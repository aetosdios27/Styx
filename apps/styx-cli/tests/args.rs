use std::path::PathBuf;

use clap::Parser;
use styx_cli::args::{Cli, Command};

#[test]
fn cli_defaults_to_interactive_tui() {
    let cli = Cli::parse_from(["styx-cli"]);

    assert!(!cli.headless);
    assert!(cli.command.is_none());
}

#[test]
fn cli_parses_headless_flag() {
    let cli = Cli::parse_from(["styx-cli", "--headless"]);

    assert!(cli.headless);
}

#[test]
fn cli_parses_add_command_with_destination() {
    let cli = Cli::parse_from([
        "styx-cli",
        "add",
        "tests/fixtures/single_file.torrent",
        "--destination",
        "/tmp/downloads",
    ]);

    assert_eq!(
        cli.command,
        Some(Command::Add {
            source: PathBuf::from("tests/fixtures/single_file.torrent"),
            destination: Some(PathBuf::from("/tmp/downloads")),
        })
    );
}

#[test]
fn cli_parses_ipc_status_command() {
    let cli = Cli::parse_from(["styx-cli", "--ipc", "/tmp/styx.sock", "status"]);

    assert_eq!(cli.ipc, Some(PathBuf::from("/tmp/styx.sock")));
    assert_eq!(cli.command, Some(Command::Status));
}
