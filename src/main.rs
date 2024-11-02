use std::{collections::HashMap, sync::Arc};

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct TestRoute {
    name: String,
    stops: Vec<TestStop>
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct TestStop {
    name: String,
}

// fn create_route(repo: &Repository, route: TestRoute, seen_stops: &mut HashMap<String, (Oid, TestRoute)>) {
//     let first_stop_name = route.stops.get(0).unwrap().name.clone();

//     let sig = repo.signature().unwrap();
//     let tree_id = repo.index().unwrap().write_tree().unwrap();
//     let tree = repo.find_tree(tree_id).unwrap();

//     let mut commit_id = repo.commit(
//         Some(format!("refs/heads/{}", route.name).as_str()),
//         &sig,
//         &sig,
//         format!("{}", first_stop_name).as_str(),
//         &tree,
//         &[]
//     ).unwrap();

//     seen_stops.insert(first_stop_name.clone(), (commit_id, route.clone()));

//     let mut in_other_line = false;
//     let mut other_line: String = "".into();
//     for i in 1..route.stops.len() {
//         let stop = route.stops.get(i).unwrap();
//         let stop_name = stop.name.clone();

//         if (seen_stops.contains_key(&stop_name)) {
//             if !in_other_line {
//                 let (target_commit_id, target_route) = seen_stops.get(&stop_name).unwrap().clone();
//                 // change the 
//                 other_line = target_route.name.clone();
//             }
//             continue;
//         } else {
//             let commit = repo.find_commit(commit_id).unwrap();
//             let tree_id = repo.index().unwrap().write_tree().unwrap();
//             let tree = repo.find_tree(tree_id).unwrap();

//             commit_id = repo.commit(
//                 Some(format!("refs/heads/{}", route.name).as_str()),
//                 &sig,
//                 &sig,
//                 format!("{}", stop_name).as_str(),
//                 &tree,
//                 &[&commit]
//             ).unwrap();

//             seen_stops.insert(stop_name.clone(), (commit_id, route.clone()));
//         }
//     }

// }

fn create_route_until_merge<'a>(
    repo: &Repository,
    stops_taken_by_routes: &HashMap<String, Vec<TestRoute>>,
    route: &TestRoute,
    from: usize,
    from_commit: Option<Oid>
) -> Option<(usize, Oid)> {
    if from >= route.stops.len() {
        return None;
    }

    if from > 0 {
        let stop = route.stops.get(from).unwrap();
        let stop_name = stop.name.clone();
        if stops_taken_by_routes.contains_key(&stop_name) {
            return Some((from, from_commit.unwrap()));
        }
    }

    let first_stop_name = route.stops.get(from).unwrap().name.clone();
    println!("Creating stop {} for route {}", first_stop_name, route.name);

    let sig = repo.signature().unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();

    let parent: Vec<Commit> =
        if let Some(from_commit) = from_commit {
            let commit = repo.find_commit(from_commit).unwrap();
            vec![commit]
        } else {
            vec![]
        };

    let parent_refs: Vec<&Commit> = parent.iter().collect();

    let mut commit_id = repo.commit(
        Some(format!("refs/heads/{}", route.name).as_str()),
        &sig,
        &sig,
        format!("{}", first_stop_name).as_str(),
        &tree,
        &parent_refs
    ).unwrap();

    for i in (from+1)..route.stops.len() {
        let stop = route.stops.get(i).unwrap();
        let stop_name = stop.name.clone();

        if stops_taken_by_routes.contains_key(&stop_name) {
            return Some((i, commit_id));
        } 

        let commit = repo.find_commit(commit_id).unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();

        println!("Adding new commit for stop {} for route {}", stop_name, route.name);
        commit_id = repo.commit(
            Some(format!("refs/heads/{}", route.name).as_str()),
            &sig,
            &sig,
            format!("{}", stop_name).as_str(),
            &tree,
            &[&commit]
        ).unwrap();
    };

    None
}

fn build_repository() {
    println!("Creating the Git repository in {}", "./result");
    let repo = Repository::init("./result").unwrap();
    println!("Repository created");

    let example_routes = vec![
        TestRoute {
            name: "Route-1".into(),
            stops: vec![
                TestStop { name: "Stop 1".into() },
                TestStop { name: "Stop 1.1".into() },
                TestStop { name: "Stop 2".into() },
                TestStop { name: "Stop 3".into() },
                TestStop { name: "Stop 10".into() },
            ]
        },
        TestRoute {
            name: "Route-2".into(),
            stops: vec![
                TestStop { name: "Stop 4".into() },
                TestStop { name: "Stop 4.1".into() },
                TestStop { name: "Stop 2".into() },
                TestStop { name: "Stop 4".into() },
                TestStop { name: "Stop 5".into() },
            ]
        },
    ];

    let mut stops_taken_by_routes: HashMap<String, Vec<TestRoute>> = HashMap::new();
    
    for route in &example_routes {
        let route = route.clone();
        for stop in &route.stops {
            stops_taken_by_routes.entry(stop.name.clone()).or_insert(vec![]).push(route.clone());
        }
    }

    let mut stops_taken_by_routes: HashMap<String, Vec<TestRoute>> = stops_taken_by_routes.into_iter().filter(|(_, routes)| routes.len() > 1).collect();

    // This is the list of routes that have been created until a certain stop
    let mut created_until_map: HashMap<String, (usize, Oid)> = HashMap::new();

    for route in &example_routes {
        if let Some((created_until, created_until_oid)) = create_route_until_merge(&repo, &stops_taken_by_routes, route, 0, None) {
            println!("Route {} created until stop {}", route.name, route.stops.get(created_until).unwrap().name);
            created_until_map.insert(route.name.clone(), (created_until, created_until_oid));
        } else {
            println!("Route {} created until the end", route.name);
            created_until_map.remove(&route.name);
        }
    }

    loop {
        let created_until_map_copy = created_until_map.clone();

        // Find the dependencies requires to build a stop
        let mut dependencies: HashMap<String, Vec<(usize, Oid, TestRoute)>> = HashMap::new();
        for tries in &created_until_map_copy {
            let (created_until, created_until_oid) = tries.1;
            let route = example_routes.iter().find(|&e| e.name.as_str() == tries.0.as_str()).unwrap();
            let stop = route.stops.get(*created_until).unwrap();
            let stop_name = stop.name.clone();
            dependencies.entry(stop_name).or_insert(vec![]).push((*created_until, *created_until_oid, route.clone()));
        }

        println!("Dependencies: {:?}", dependencies);

        if dependencies.len() == 0 {
            break;
        }

        for (stop_name, dependencies) in dependencies {
            let mut parents = dependencies.iter().map(|e| e.1).collect::<Vec<_>>();
            parents.dedup();
            let parents: Vec<Commit> = parents.iter().map(|e| repo.find_commit(e.clone()).unwrap()).collect();
            let parents_ref = parents.iter().collect::<Vec<&Commit>>();

            let route_that_will_be_host = dependencies.first().unwrap().2.clone();

            let sig = repo.signature().unwrap();
            let tree_id = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();

            println!("Creating common stop for {} and lines {}", stop_name, dependencies.iter().map(|e| e.2.name.clone()).collect::<Vec<_>>().join(", "));
            let commit_id = repo.commit(
                Some(format!("refs/heads/{}", route_that_will_be_host.name).as_str()),
                &sig,
                &sig,
                format!("{} from merge", stop_name).as_str(),
                &tree,
                &parents_ref
            ).unwrap();

            // Move refs of the routes to that commit
            for (_, _, route) in dependencies {
                repo.reference(format!("refs/heads/{}", route.name).as_str(), commit_id, true, "Moving the ref to the merge commit").unwrap();
            }


            created_until_map.clear();

            for tries in &created_until_map_copy {
                for route in &example_routes {
                    if route.name.as_str() == tries.0.as_str() {
                        println!("Trying to create stop for route {:?}, stopId: ${:?}", route.name, route.stops.get(tries.1.0).unwrap());
                        if let Some((created_until, created_until_oid)) = create_route_until_merge(&repo, &stops_taken_by_routes, route, tries.1.0 + 1, Some(commit_id)) {
                            println!("Route {} created until stop {}", route.name, route.stops.get(created_until).unwrap().name);
                            created_until_map.insert(route.name.clone(), (created_until, created_until_oid));
                        } else {
                            println!("Route {} created until the end", route.name);
                        }
                    }
                }
            }
        }
    }
}


fn main() {
    build_repository();
    return ();

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

    let mut satisfied = false;

    while !satisfied {
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
        
        satisfied = confirm.unwrap();
    }

}
