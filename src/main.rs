use std::{collections::{HashMap, HashSet}, sync::Arc};

use clap::Parser;
use git2::{build, Commit, Oid, Repository};
use gtfs_structures::{Gtfs, Route, Stop};
use inquire::{
    formatter::MultiOptionFormatter, list_option::ListOption, validator::{MultiOptionValidator, Validation}, Confirm, MultiSelect
};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The URL or path to the folder containing the GTFS files
    #[arg(short, long, default_value = "./gtfs")]
    path: String,
    /// The directory where to create the Git repository
    #[arg(short, long, default_value = "./result")]
    git_dir: String
}

type RouteId = String;
type StopId = String;
type RouteName = String;
type StopName = String;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct GitRoute {
    id: RouteId,
    name: RouteName,
    stops: Vec<GitStop>
}

impl GitRoute {
    pub fn stops(&self) -> &Vec<GitStop> {
        &self.stops
    }

    pub fn stop(&self, idx: usize) -> Option<&GitStop> {
        self.stops.get(idx)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct GitStop {
    id: StopId,
    name: StopName,
}

fn initialize_repo() -> Repository {
    println!("Creating the Git repository in {}", "./result");
    let repo = Repository::init("./result").unwrap();
    println!("Repository created");
    repo
}

fn add_commit_to_head(repo: &Repository, branch: &str, commit: Oid) {
    repo.reference(format!("refs/heads/{}", branch).as_str(), commit, true, "Moving the ref to the merge commit").unwrap();
}

fn commit(repo: &Repository, message: &str, parents: Vec<Oid>, branch: &str) -> Oid {
    println!("Creating commit with message {} with parents {:?} on branch {}", message, parents, branch);
    let refs = format!("refs/heads/{}", branch);
    repo.set_head(&refs).unwrap();
    // repo.checkout_head(None).unwrap();

    let reffffs = repo.references().unwrap().map(|e| e.unwrap()).collect::<Vec<_>>();
    for ref_ in reffffs {
        println!("Ref: {:?} {:?}", ref_.name(), ref_.target());
    }

    let index = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(index).unwrap();
    let sig = repo.signature().unwrap();

    let parents: Vec<Commit> = parents.into_iter().map(|e| repo.find_commit(e).unwrap()).collect();
    let parents_refs: Vec<&Commit> = parents.iter().collect();

    let commit_id = repo.commit(
        Some(&format!("refs/heads/{}", branch)),
        &sig,
        &sig,
        message,
        &tree,
        &parents_refs
    ).unwrap();

    commit_id
}

fn get_conflicts(routes: &HashMap<RouteId, GitRoute>) -> HashMap<StopId, Vec<RouteId>> {
    let mut conflicts: HashMap<StopId, Vec<RouteId>> = HashMap::new();
    for route in routes.values() {
        for stop in route.stops() {
            conflicts.entry(stop.id.clone()).or_insert(vec![]).push(route.id.clone());
        }
    }

    conflicts.into_iter().filter(|(_, routes)| routes.len() > 1).collect()
}

fn build_route_alone(repo: &Repository, route: &GitRoute, previous: RouteBuildState, conflicts: &HashSet<StopId>) -> RouteBuildState {
    if let RouteBuildState::Built(commit) = previous {
        return RouteBuildState::Built(commit);
    };

    let from_stop_idx =
        if let RouteBuildState::Pending(idx, _, commit) = previous {
            if idx + 1 >= route.stops().len() {
                return RouteBuildState::Built(commit);
            }
            idx + 1
        } else {
            0
        };

    let mut state = previous;

    for stop_idx in from_stop_idx..route.stops().len() {
        println!("Trying to build stop {} for route {}. Current state: {:?}", stop_idx, route.name, state);
        let stop = route.stop(stop_idx).unwrap();
        if conflicts.contains(&stop.id) {
            println!("Stop {} is in conflict", stop.name);
            break;
        }

        println!("Creating stop {} for route {}", stop.name, route.name);
        let parent = if let Some(commit) = state.commit() {
            vec![commit.clone()]
        } else {
            vec![]
        };
        let commit = commit(repo, &stop.name, parent, &route.name);
        state = state.did_stop(stop_idx, commit)
    };

    state
}

#[derive(Debug, Clone)]
enum RouteBuildState {
    // Untouched, not created yet. The usize is the length of the route.
    Untouched(usize),
    Built(Oid),
    // Built until stop (index), inclusive. Has 2nd usize stops
    Pending(usize, usize, Oid)
}

impl RouteBuildState {
    fn commit(&self) -> Option<&Oid> {
        match self {
            RouteBuildState::Built(commit) => Some(commit),
            RouteBuildState::Pending(_, _, commit) => Some(commit),
            _ => None
        }
    }

    fn did_commit(self, commit: Oid) -> RouteBuildState {
        match self {
            RouteBuildState::Built(_) => panic!("The route has already been built"),
            RouteBuildState::Pending(idx, max, _) if idx == max - 2 => RouteBuildState::Built(commit),
            RouteBuildState::Pending(idx, max, _) => RouteBuildState::Pending(idx + 1, max, commit),
            RouteBuildState::Untouched(max) => RouteBuildState::Pending(0, max, commit)
        }
    }

    fn did_stop(self, index: usize, commit: Oid) -> RouteBuildState {
        match self {
            RouteBuildState::Built(_) => panic!("The route has already been built"),
            RouteBuildState::Untouched(_) if index > 0 => panic!("The route has not been built, excessive index."),
            RouteBuildState::Pending(idx, _, _) if idx + 1 != index => panic!("The stop has already been built"),
            RouteBuildState::Pending(idx, max, _) if idx >= max => panic!("The line is normally already built."),
            RouteBuildState::Pending(_, max, _) if index == max - 1 => RouteBuildState::Built(commit),
            RouteBuildState::Untouched(max) => RouteBuildState::Pending(index, max, commit),
            RouteBuildState::Pending(_, max, _) => RouteBuildState::Pending(index, max, commit)
        }
    }
}

fn initialize_states(routes: &HashMap<RouteId, GitRoute>) -> HashMap<RouteId, RouteBuildState> {
    routes.iter().map(|(id, r)| (id.clone(), RouteBuildState::Untouched(r.stops.len()))).collect()
}

fn find_dependencies(routes: &HashMap<RouteId, GitRoute>, route_to_current_commit: &HashMap<RouteId, RouteBuildState>) -> HashMap<StopId, Vec<RouteId>> {
    let mut dependencies: HashMap<StopId, Vec<RouteId>> = HashMap::new();
    for (route_id, state) in route_to_current_commit {
        if let RouteBuildState::Pending(idx, _, _) = state {
            let stop_id = routes.get(route_id).unwrap().stops().get(*idx + 1).unwrap().id.clone();
            dependencies.entry(stop_id).or_insert(vec![]).push(route_id.clone());
        }

        if let RouteBuildState::Untouched(_) = state {
            let stop_id = routes.get(route_id).unwrap().stops().get(0).unwrap().id.clone();
            dependencies.entry(stop_id).or_insert(vec![]).push(route_id.clone());
        }
    }

    dependencies
}

fn build_repository(routes: HashMap<RouteId, GitRoute>) {
    let repo = initialize_repo();

    let conflicts: HashMap<StopId, Vec<RouteId>> = get_conflicts(&routes);
    let mut states: HashMap<RouteId, RouteBuildState> = initialize_states(&routes);

    println!("Conflicts: {:?}", conflicts);

    // Bootstrap the routes
    for route in &routes {
        println!("Building route {}", route.1.name);
        let state = states.get(route.0).unwrap();
        let state = build_route_alone(&repo, &route.1, state.clone(), &conflicts.keys().cloned().collect());
        println!("New state for route {}: {:?}", route.1.name, state);
        states.insert(route.0.clone(), state);
    }

    // Until all dependencies are solved
    loop {
        println!("Entering conflict mode…");
        // Find the dependencies required to build a stop
        let dependencies = find_dependencies(&routes, &states);

        if dependencies.len() == 0 {
            break;
        }

        println!("Dependencies: {:?}", dependencies);
        let mut built_something = false;

        for (dep_stop_id, dep_routes) in dependencies {
            let target = conflicts.get(&dep_stop_id).unwrap();
            // We have not built all the dependencies yet
            if target.len() != dep_routes.len() {
                println!("Not all dependencies have been built yet for stop {}", dep_stop_id);
                continue;
            }

            let stop_name = routes.get(target.first().unwrap()).unwrap().stops().iter().find(|e| e.id == dep_stop_id).unwrap().name.clone();
            println!("Creating common stop for {} and lines {}", stop_name, dep_routes.iter().map(|e| routes.get(e).unwrap().name.clone()).collect::<Vec<_>>().join(", "));
            // build common stop

            // Choose a route's branch to put all the commits
            let host_route = dep_routes.first().unwrap();
            let host_route_name = routes.get(host_route).unwrap().name.as_str();
            let other_routes = dep_routes.iter().skip(1).collect::<Vec<_>>();

            println!("Host route: {}", routes.get(host_route).unwrap().name);

            // Get all their states, to get their oid
            let routes_state = states.iter().filter(|(id, _)| dep_routes.contains(id)).map(|(id, state)| {
                match state {
                    RouteBuildState::Pending(_, _, _) => (id.clone(), state.clone()),
                    RouteBuildState::Built(_) => panic!("The route has already been built"),
                    RouteBuildState::Untouched(_) => (id.clone(), state.clone())
                }
            }).collect::<HashMap<_, _>>();
            println!("Preparing commit…");
            let parents = routes_state.iter()
                .map(|(_, state)| state.commit())
                .filter_map(|e| e)
                .map(|e| e.clone())
                .collect::<Vec<_>>();
            let commit = commit(&repo, &format!("{} from merge", stop_name), parents, host_route_name);
            // advance heads of the other routes
            for route in other_routes {
                let route = routes.get(route).unwrap().name.as_str();
                add_commit_to_head(&repo, route, commit);
            }

            println!("Commit created");

            built_something = true;

            println!("Updating states…");
            for (route, prev_state) in routes_state {
                println!("Updating state for route {}, from {:?}, to {:?}", route, prev_state, prev_state.clone().did_commit(commit));
                states.insert(route.clone(), prev_state.clone().did_commit(commit));
            }

            // Continue building the routes
            println!("Finished solving the conflict, continuing building the routes…");
            for route in dep_routes {
                let route = routes.get(&route).unwrap();
                println!("Building route {}", route.name);
                let state = states.get(&route.id).unwrap();
                let state = build_route_alone(&repo, &route, state.clone(), &conflicts.keys().cloned().collect());
                states.insert(route.id.clone(), state);
            }
        }

        if !built_something {
            panic!("Infinite loop detected.");
        }
    }
    

}

// fn create_route_until_merge<'a>(
//     repo: &Repository,
//     stops_taken_by_routes: &HashMap<String, Vec<GitRoute>>,
//     route: &GitRoute,
//     from: usize,
//     from_commit: Option<Oid>
// ) -> Option<(usize, Oid)> {
//     if from >= route.stops.len() {
//         return None;
//     }

//     if from > 0 {
//         let stop: &GitStop = route.stops.get(from).unwrap();
//         let stop_id = stop.id.clone();
//         if stops_taken_by_routes.contains_key(&stop_id) {
//             return Some((from, from_commit.unwrap()));
//         }
//     }

//     let first_stop_name = route.stops.get(from).unwrap().name.clone();
//     println!("Creating stop {} for route {}", first_stop_name, route.name);

//     let sig = repo.signature().unwrap();
//     let tree_id = repo.index().unwrap().write_tree().unwrap();
//     let tree = repo.find_tree(tree_id).unwrap();

//     let parent: Vec<Commit> =
//         if let Some(from_commit) = from_commit {
//             let commit = repo.find_commit(from_commit).unwrap();
//             vec![commit]
//         } else {
//             vec![]
//         };

//     let parent_refs: Vec<&Commit> = parent.iter().collect();

//     let mut commit_id = repo.commit(
//         Some(format!("refs/heads/{}", route.name).as_str()),
//         &sig,
//         &sig,
//         format!("{}", first_stop_name).as_str(),
//         &tree,
//         &parent_refs
//     ).unwrap();

//     for i in (from+1)..route.stops.len() {
//         let stop = route.stops.get(i).unwrap();
//         let stop_id = stop.id.clone();
//         let stop_name = stop.name.clone();

//         if stops_taken_by_routes.contains_key(&stop_id) {
//             return Some((i, commit_id));
//         } 

//         let commit = repo.find_commit(commit_id).unwrap();
//         let tree_id = repo.index().unwrap().write_tree().unwrap();
//         let tree = repo.find_tree(tree_id).unwrap();

//         println!("Adding new commit for stop {} for route {}", stop_name, route.name);
//         commit_id = repo.commit(
//             Some(format!("refs/heads/{}", route.name).as_str()),
//             &sig,
//             &sig,
//             format!("{}", stop_name).as_str(),
//             &tree,
//             &[&commit]
//         ).unwrap();
//     };

//     None
// }

// fn build_repository(example_routes: Vec<GitRoute>) {
//     println!("Creating the Git repository in {}", "./result");
//     let repo = Repository::init("./result").unwrap();
//     println!("Repository created");

//     let mut stops_taken_by_routes: HashMap<String, Vec<GitRoute>> = HashMap::new();
    
//     for route in &example_routes {
//         let route = route.clone();
//         for stop in &route.stops {
//             stops_taken_by_routes.entry(stop.id.clone()).or_insert(vec![]).push(route.clone());
//         }
//     }
//     println!("Stops taken by routes: {:?}", stops_taken_by_routes);

//     let mut stops_taken_by_routes: HashMap<String, Vec<GitRoute>> = stops_taken_by_routes.into_iter().filter(|(_, routes)| routes.len() > 1).collect();
//     println!("Stops taken by multiple routes: {:?}", stops_taken_by_routes);

//     // This is the list of routes that have been created until a certain stop
//     let mut created_until_map: HashMap<String, (usize, Oid)> = HashMap::new();

//     for route in &example_routes {
//         if let Some((created_until, created_until_oid)) = create_route_until_merge(&repo, &stops_taken_by_routes, route, 0, None) {
//             println!("Route {} created until stop {}", route.name, route.stops.get(created_until).unwrap().name);
//             created_until_map.insert(route.id.clone(), (created_until, created_until_oid));
//         } else {
//             println!("Route {} created until the end", route.name);
//             created_until_map.remove(&route.id);
//         }
//     }

//     loop {
//         let created_until_map_copy = created_until_map.clone();

//         // Find the dependencies requires to build a stop
//         let mut dependencies: HashMap<String, Vec<(usize, Oid, GitRoute)>> = HashMap::new();
//         for tries in &created_until_map_copy {
//             let (created_until, created_until_oid) = tries.1;
//             let route = example_routes.iter().find(|&e| e.id.as_str() == tries.0.as_str()).unwrap();
//             let stop = route.stops.get(*created_until).unwrap();
//             let stop_id = stop.id.clone();
//             dependencies.entry(stop_id).or_insert(vec![]).push((*created_until, *created_until_oid, route.clone()));
//         }

//         if dependencies.len() == 0 {
//             break;
//         }

//         for (stop_id, dependencies) in dependencies {
//             let stop_name = {
//                 let (idx, _, route) = dependencies.first().unwrap().clone();
//                 route.stops.get(idx).unwrap().name.clone()
//             };

//             let mut parents = dependencies.iter().map(|e| e.1).collect::<Vec<_>>();
//             parents.dedup();
//             let parents: Vec<Commit> = parents.iter().map(|e| repo.find_commit(e.clone()).unwrap()).collect();
//             let parents_ref = parents.iter().collect::<Vec<&Commit>>();

//             let route_that_will_be_host = dependencies.first().unwrap().2.clone();

//             let sig = repo.signature().unwrap();
//             let tree_id = repo.index().unwrap().write_tree().unwrap();
//             let tree = repo.find_tree(tree_id).unwrap();

//             println!("Creating common stop for {} and lines {}", stop_name, dependencies.iter().map(|e| e.2.name.clone()).collect::<Vec<_>>().join(", "));
//             let commit_id = repo.commit(
//                 Some(format!("refs/heads/{}", route_that_will_be_host.name).as_str()),
//                 &sig,
//                 &sig,
//                 format!("{} from merge", stop_name).as_str(),
//                 &tree,
//                 &parents_ref
//             ).unwrap();

//             // Move refs of the routes to that commit
//             for (_, _, route) in dependencies {
//                 repo.reference(format!("refs/heads/{}", route.name).as_str(), commit_id, true, "Moving the ref to the merge commit").unwrap();
//             }


//             created_until_map.clear();

//             for tries in &created_until_map_copy {
//                 for route in &example_routes {
//                     if route.id.as_str() == tries.0.as_str() {
//                         println!("Trying to create stop for route {:?}, stopId: ${:?}", route.name, route.stops.get(tries.1.0).unwrap());
//                         if let Some((created_until, created_until_oid)) = create_route_until_merge(&repo, &stops_taken_by_routes, route, tries.1.0 + 1, Some(commit_id)) {
//                             println!("Route {} created until stop {}", route.name, route.stops.get(created_until).unwrap().name);
//                             created_until_map.insert(route.name.clone(), (created_until, created_until_oid));
//                         } else {
//                             println!("Route {} created until the end", route.name);
//                         }
//                     }
//                 }
//             }
//         }
//     }
// }


fn main() {
    let validator = |a: &[ListOption<&Route>]| {
        if a.len() == 0 {
            return Ok(Validation::Invalid("At least one route must be selected".into()))
        } else {
            return Ok(Validation::Valid)
        }
    };

    let args = Args::parse();
    println!("Reading the GTFS files from {}. This might take a while…", args.path);
    let gtfs = Gtfs::new(&args.path).unwrap();
    let routes = gtfs.routes.clone();
    let routes = routes.into_iter().map(|(_, route)| (route)).collect::<Vec<_>>();
    println!("Found {} routes", routes.len());


    let selected_routes = loop {
        let selected_routes = MultiSelect::new("Select the routes you want to include in the repository", routes.clone())
            .with_validator(validator)
            .prompt()
            .unwrap();

        println!("Selected routes: ");
        for route in &selected_routes {
            // TODO: make that better
            let g = &gtfs;
            let t = &g.trips;
            let trip = t.into_iter().find(|(_, trip)| trip.route_id == route.id).map(|(_, trip)| trip);

            if let Some(trip) = trip {
                println!("{}: From {:?} to {:?}", route, trip.stop_times.first().map(|e| e.stop.name.clone()).flatten(), trip.stop_times.last().map(|e| e.stop.name.clone()).flatten());
            } else {
                println!("{}: No trips found", route);
            }

        }

        let confirm = Confirm::new("Are you satisfied with the selection?")
            .with_default(false)
            .prompt();

        if let Ok(true) = confirm {
            break selected_routes;
        }
    };

    // Build our internal data-structure
    let mut git_routes: HashMap<RouteId, GitRoute> = HashMap::new();

    for route in &selected_routes {
        let g = &gtfs;
        let t = &g.trips;
        let trip = t.into_iter().find(|(_, trip)| trip.route_id == route.id).map(|(_, trip)| trip);

        if let Some(trip) = trip {
            let stops = trip.stop_times.iter().map(|e| {
                let name = if let Some(name) = e.stop.name.clone() {
                    name.clone()
                } else {
                    e.stop.id.clone()
                };

                GitStop {
                    id: e.stop.id.clone(),
                    name: name
                }
            }).collect::<Vec<_>>();

            let route_name = if let Some(name) = route.long_name.clone() {
                name
            } else {
                if let Some(short_name) = route.short_name.clone() {
                    short_name.clone()
                } else {
                    route.id.clone()
                }
            };

            git_routes.insert(route.id.clone(), GitRoute {
                id: route.id.clone(),
                name: route_name,
                stops
            });
        }
    }

    build_repository(git_routes);
}
