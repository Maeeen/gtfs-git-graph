# Preview your favorite bus, tram, trains and metro stops in a Git

Example: [Romandie, Switzerland](https://github.com/Maeeen/vaud-rer-git), [graph](https://github.com/Maeeen/vaud-rer-git/network)

In the same mindset as [MetroGit](https://github.com/vbarbaresi/MetroGit), this
project generalizes the idea to any public transport that publishes their data.

Currently, this project supports the GTFS format, which is a standard format for
public transportation schedules and associated geographic information.

This has not been extensively **tested**, so be aware that bugs, excessive logs
and crashes are to be expected.

## How to use

1. Run it with [cargo](https://github.com/rust-lang/cargo) and specify the path
to your GTFS folder:

```sh
cargo run --release -- --path ./gtfs
```

Technically, the [gtfs_structure](https://github.com/rust-transit/gtfs-structure)
allows you to load the GTFS data from a ZIP file and a URL, but this has not been
tested.

You can specify the directory where the repository will be created with the
`--git-dir` flag.

If your GTFS data are too big and the filter is too slow, you can use the
`--prefilter <line1>,<line2>,…` flag to only load the data for the specified
lines.

2. Select your lines in the CLI, confirm.

3. Preview the repository in your favorite Git client.

## To know

* Circular lines are not supported (and not planned to be supported. I don't know
how to represent them in a Git repository).
* The Rust code is not the cleanest, and can crash at anytime.