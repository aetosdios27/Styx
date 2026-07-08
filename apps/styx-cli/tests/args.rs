use std::path::PathBuf;

use clap::Parser;
use styx_cli::args::{Cli, Command, DaemonCommand};

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

#[test]
fn cli_parses_smoke_command() {
    let cli = Cli::parse_from([
        "styx-cli",
        "smoke",
        "--torrent",
        "ubuntu.torrent",
        "--dest",
        "/tmp/styx-smoke",
        "--listen-port",
        "6999",
    ]);

    assert_eq!(
        cli.command,
        Some(Command::Smoke {
            torrent: PathBuf::from("ubuntu.torrent"),
            dest: PathBuf::from("/tmp/styx-smoke"),
            listen_port: 6999,
        })
    );
}

#[test]
fn cli_parses_download_command() {
    let cli = Cli::parse_from([
        "styx-cli",
        "download",
        "--torrent",
        "ubuntu.torrent",
        "--dest",
        "/tmp/styx-download",
        "--listen-port",
        "6999",
    ]);

    assert_eq!(
        cli.command,
        Some(Command::Download {
            torrent: PathBuf::from("ubuntu.torrent"),
            dest: PathBuf::from("/tmp/styx-download"),
            listen_port: 6999,
        })
    );
}

#[test]
fn cli_parses_daemon_start_command() {
    let cli = Cli::parse_from([
        "styx-cli",
        "daemon",
        "start",
        "--state-dir",
        "/tmp/styx-state",
        "--socket",
        "/tmp/styx.sock",
    ]);

    assert_eq!(
        cli.command,
        Some(Command::Daemon(DaemonCommand::Start {
            state_dir: PathBuf::from("/tmp/styx-state"),
            socket: PathBuf::from("/tmp/styx.sock"),
        }))
    );
}

#[test]
fn cli_parses_daemon_status_command() {
    let cli = Cli::parse_from(["styx-cli", "daemon", "status", "--socket", "/tmp/styx.sock"]);

    assert_eq!(
        cli.command,
        Some(Command::Daemon(DaemonCommand::Status {
            socket: PathBuf::from("/tmp/styx.sock"),
        }))
    );
}

#[test]
fn cli_parses_daemon_stop_command() {
    let cli = Cli::parse_from(["styx-cli", "daemon", "stop", "--socket", "/tmp/styx.sock"]);

    assert_eq!(
        cli.command,
        Some(Command::Daemon(DaemonCommand::Stop {
            socket: PathBuf::from("/tmp/styx.sock"),
        }))
    );
}
