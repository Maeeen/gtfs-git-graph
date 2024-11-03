use std::collections::{HashMap, HashSet};

use clap::Parser;
use git2::{Commit, Oid, Repository};
use gtfs_structures::{Gtfs, Route, Trip};
use inquire::{
    list_option::ListOption, validator::Validation, Confirm, MultiSelect
};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The URL or path to the folder containing the GTFS files
    #[arg(short, long, default_value = "./gtfs")]
    path: String,
    /// The directory where to create the Git repository
    #[arg(short, long, default_value = "./result")]
    git_dir: String,

    /// To prefilter routes names, if the CLI is too slow
    #[arg(short, long, default_value = "")]
    prefilter: String
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
        let commit = commit(repo, &format!("{}", &stop.name), parent, &route.name);
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

    for (stop_id, routes) in dependencies.clone() {
        if routes.len() == 1 {
            dependencies.remove(&stop_id);
        }
    }

    dependencies
}

/// Does not 
fn fix_order(routes: HashMap<RouteId, GitRoute>) -> HashMap<RouteId, GitRoute> {
    let routes = routes;

    fn same_order(a: &GitRoute, b: &GitRoute) -> bool {
        // Make sure that both routes take the stops in the same order

        // All A's stops
        let a_stops = a.stops().iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        // All B's stops that are in A 
        let common_stops = b.stops().iter().filter(|s| a_stops.contains(&s.id)).map(|e| e.id.clone()).collect::<Vec<_>>();
        // All A's stops that are common
        let a_stops = a_stops.iter().filter(|e| common_stops.contains(e)).map(|e| e.clone()).collect::<Vec<_>>();
    
        a_stops == common_stops
    }

    let mut reference_routes: Vec<(RouteId, GitRoute)> = Vec::new();

    for route in routes.clone() {
        if reference_routes.len() == 0 {
            reference_routes.push(route);
            continue;
        }

        // consider route 1, does it have same order with all previous routes?
        if reference_routes.iter().all(|e| same_order(&route.1, &e.1)) {
            reference_routes.push(route);
        } else {
            // otherwise, if flipped, does it have same order with all previous routes?
            let mut flipped = route.1.clone();
            flipped.stops.reverse();
            if reference_routes.iter().all(|e| same_order(&flipped, &e.1)) {
                reference_routes.push((route.0.clone(), flipped));
            } else {
                // Okay, we can't do anymore, a reference route is being in the wrong order…
                // Shit.

                // Who's being a naughty boy here in our reference routes?
                let naughty_boys = reference_routes.iter().filter(|e| !same_order(&flipped, &e.1)).collect::<Vec<_>>();
                // can we flip the naughty boys?
                let flipped_naughty = naughty_boys.iter().map(|e| {
                    let mut flipped = e.1.clone();
                    flipped.stops.reverse();
                    (e.0.clone(), flipped)
                }).collect::<HashMap<_, _>>();

                // proposal for new reference routes
                let mut new_reference = reference_routes.clone().into_iter().filter(|e| !flipped_naughty.contains_key(&e.0)).collect::<Vec<_>>(); // not naughty ones
                for naughty in flipped_naughty {
                    new_reference.push(naughty.clone());
                }
                // check if all of those are valid

                // Verify that there are no more conflicts with current addition
                if new_reference.iter().all(|e| same_order(&route.1, &e.1)) {
                    // verify that there is no more conflicts between each routes
                    let mut successful_proposal = true;
                    for r1 in &new_reference {
                        for r2 in &new_reference {
                            if r1.0 == r2.0 {
                                continue;
                            }
                            if !same_order(&r1.1, &r2.1) {
                                successful_proposal = false;
                                break;
                            }
                        }
                    }
                    if successful_proposal {
                        reference_routes = new_reference;
                        reference_routes.push(route);
                        continue;
                    }
                }


                println!("Could not unify stops order for route {}. Details:", route.1.name);
                println!("Stops for route {}: {:?}", route.1.name, route.1.stops.iter().map(|e| e.name.clone()).collect::<Vec<_>>());
                for e in &reference_routes {
                    println!("({:?}) {}: {:?}", same_order(&route.1, &e.1), e.1.name, e.1.stops.iter().map(|e| e.name.clone()).collect::<Vec<_>>());
                    println!("({:?},R) {}: {:?}", same_order(&flipped, &e.1), e.1.name, e.1.stops.iter().map(|e| e.name.clone()).collect::<Vec<_>>());
                }
                panic!("Could not unify stops order for route {}", route.1.name);
            }
        }

    }

    println!("Decided order:");
    for route in &reference_routes {
        println!("{}: {:?}", route.1.name, route.1.stops.iter().map(|e| e.name.clone()).collect::<Vec<_>>());
    }


    reference_routes.into_iter().collect()
}

fn build_repository(routes: HashMap<RouteId, GitRoute>) {
    let repo = initialize_repo();

    println!("Fixing order of the routes…");
    let routes = fix_order(routes);

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
        if states.iter().all(|(_, state)| matches!(state, RouteBuildState::Built(_))) {
            println!("All routes have been built");
            break;
        }

        println!("Checking for conflicts…");
        // Find the dependencies required to build a stop
        let dependencies = find_dependencies(&routes, &states);


        println!("Entering conflict mode…");

        println!("Dependencies: {:?}", dependencies);
        let mut built_something = false;

        for (dep_stop_id, dep_routes) in dependencies {
            
            let target = conflicts.get(&dep_stop_id).unwrap();
            let stop_name = routes.get(target.first().unwrap()).unwrap().stops().iter().find(|e| e.id == dep_stop_id).unwrap().name.clone();
            // We have not built all the dependencies yet
            if target.len() != dep_routes.len() {
                println!("Not all dependencies have been built yet for stop {} ({})", stop_name, dep_stop_id);
                continue;
            }

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
            let mut parents: Vec<Oid> = Vec::new();
            for dep_route in &dep_routes {
                let state = routes_state.get(dep_route).unwrap();
                if let Some(commit) = state.commit() {
                    parents.push(commit.clone());
                }
            } 
            let commit = commit(&repo, &format!("{}", stop_name), parents, host_route_name);
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
            println!("Infinite loop detected. Done until this:");
            for route_state in &states {
                let route_name = routes.get(route_state.0).unwrap().name.clone();
                let start_stop = routes.get(route_state.0).unwrap().stops().first().unwrap().name.clone();
                let end_stop = routes.get(route_state.0).unwrap().stops().last().unwrap().name.clone();
                let state = match route_state.1 {
                    RouteBuildState::Built(_) => format!("{} Built ({} to {})", route_name, start_stop, end_stop),
                    RouteBuildState::Pending(idx, _, _) => {
                        let stops = routes.get(route_state.0).unwrap().stops();
                        let done_stop = stops.get(*idx).unwrap();
                        let waiting_stop = stops.get(idx + 1);
                        format!("{} Done until stop {} (included), waiting for {:?}", route_name, done_stop.name, waiting_stop)
                    },
                    RouteBuildState::Untouched(_) => format!("{} Not started ({} to {})", route_name, start_stop, end_stop)
                };
                println!("{:?}", state);
            }
            panic!("Infinite loop detected");
        }
    }
    

}

#[derive(Debug, Clone)]
struct RouteDisplayWrapper(Route, Trip);

impl std::fmt::Display for RouteDisplayWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let route = &self.0;
        let trip = &self.1;
        let from = trip.stop_times.first().map(|e| e.stop.name.clone()).flatten();
        let to = trip.stop_times.last().map(|e| e.stop.name.clone()).flatten();
        write!(f, "{}: From {:?} to {:?}", route, from, to)
    }
}

fn main() {
    let validator = |a: &[ListOption<&RouteDisplayWrapper>]| {
        if a.len() == 0 {
            return Ok(Validation::Invalid("At least one route must be selected".into()))
        } else {
            return Ok(Validation::Valid)
        }
    };

    let args = Args::parse();
    let filter_lines = args.prefilter.split(",").collect::<HashSet<_>>();

    println!("Reading the GTFS files from {}. This might take a while…", args.path);
    let gtfs = Gtfs::new(&args.path).unwrap();
    let routes = gtfs.routes;
    let trips = gtfs.trips;
    println!("Found {} routes", routes.len());
    println!("Found {} trips", trips.len());
    let routes = {
        trips.iter().map(|(_, trip)| {
            let route_id = trip.route_id.clone();
            let route = routes.get(&route_id).unwrap();
            if let Some(long_name) = route.long_name.as_ref() {
                if filter_lines.len() > 0 && !filter_lines.contains(long_name.as_str()) {
                    return None;
                }
            }
            if let Some(short_name) = route.short_name.as_ref() {
                if filter_lines.len() > 0 && !filter_lines.contains(short_name.as_str()) {
                    return None;
                }
            }
            Some(RouteDisplayWrapper(route.clone(), trip.clone()))
        }).flatten().collect::<Vec<_>>()
    };


    let selected_routes = loop {
        let selected_routes = MultiSelect::new("Select the routes you want to include in the repository", routes.clone())
            .with_validator(validator)
            .prompt()
            .unwrap();

        println!("Selected routes: ");
        for route in &selected_routes {
            let trip = &route.1;
            println!("{}: From {:?} to {:?}", route, trip.stop_times.first().map(|e| e.stop.name.clone()).flatten(), trip.stop_times.last().map(|e| e.stop.name.clone()).flatten());
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

    for route in selected_routes {
        let trip = route.1;

        let stops = trip.stop_times.iter().map(|e| {
            let name = if let Some(name) = e.stop.name.clone() {
                name.clone()
            } else {
                e.stop.id.clone()
            };

            let id = e.stop.id.clone();
            let id = id.split(":").collect::<Vec<_>>().first().unwrap().to_string();

            GitStop {
                id: id,
                name: name
            }
        }).collect::<Vec<_>>();

        let route_name = if let Some(name) = route.0.long_name.clone() {
            name
        } else {
            if let Some(short_name) = route.0.short_name.clone() {
                short_name.clone()
            } else {
                route.0.id.clone()
            }
        };

        git_routes.insert(route.0.id.clone(), GitRoute {
            id: route.0.id.clone(),
            name: route_name,
            stops
        });
    }

    build_repository(git_routes);
}
