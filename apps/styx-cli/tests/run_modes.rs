use clap::Parser;
use styx_cli::{args::Cli, run_command_once};

#[test]
fn direct_status_command_writes_success_json() {
    let cli = Cli::parse_from(["styx-cli", "status"]);
    let mut output = Vec::new();

    run_command_once(cli, &mut output).unwrap();

    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["ok"], true);
}

#[test]
fn invalid_hash_command_returns_error() {
    let cli = Cli::parse_from(["styx-cli", "pause", "bad"]);
    let mut output = Vec::new();

    let err = run_command_once(cli, &mut output).unwrap_err();

    assert!(err.to_string().contains("40 hex characters"));
}
