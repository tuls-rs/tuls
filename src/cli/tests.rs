use super::{Cli, Command, NetworkPolicy, RobotsPolicy};
use clap::Parser;
use std::path::PathBuf;

#[test]
fn workspace_servers_default_to_current_directory() {
    for args in [
        &["tuls", "filesystem"][..],
        &["tuls", "shell"][..],
        &["tuls", "skills"][..],
        &["tuls", "agents"][..],
    ] {
        let cli = Cli::try_parse_from(args)
            .unwrap_or_else(|error| panic!("failed to parse {args:?}: {error}"));
        match cli.command {
            Command::Filesystem(options) | Command::Shell(options) => {
                assert_eq!(options.dirs, [PathBuf::from(".")]);
            }
            Command::Skills(options) | Command::Agents(options) => {
                assert_eq!(options.dir, PathBuf::from("."));
            }
            Command::Fetch(_) | Command::Memory(_) => panic!("unexpected command"),
        }
    }
}

#[test]
fn fetch_defaults_are_safe() {
    let cli = Cli::try_parse_from(["tuls", "fetch"]).unwrap();
    let Command::Fetch(options) = cli.command else {
        panic!("unexpected command");
    };
    assert_eq!(options.robots, RobotsPolicy::Respect);
    assert_eq!(options.network, NetworkPolicy::Public);
}
