use clap::Parser;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use styx_cli::{
    args::{Cli, DaemonCommand},
    ipc::serve_daemon_socket,
    run_command_once, run_daemon_command_once,
};
use styx_runtime::{DaemonConfig, DaemonRuntime, RuntimeConfig};

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

#[test]
fn direct_add_magnet_invalid_uri_returns_error_json() {
    let cli = Cli::parse_from([
        "styx-cli",
        "add-magnet",
        "not-a-magnet",
        "--destination",
        "/tmp/downloads",
    ]);
    let mut output = Vec::new();

    run_command_once(cli, &mut output).unwrap();

    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["ok"], false);
}

#[tokio::test]
async fn daemon_status_command_writes_success_json() {
    let root = unique_temp_dir("styx-cli-daemon-status");
    let socket = root.join("styx.sock");
    let daemon = DaemonRuntime::start(daemon_config(&root, &socket))
        .await
        .unwrap();
    let server_socket = socket.clone();
    let server_daemon = daemon.clone();
    let server =
        tokio::spawn(async move { serve_daemon_socket(&server_socket, server_daemon).await });
    wait_for_socket(&socket).await;
    let mut output = Vec::new();

    run_daemon_command_once(DaemonCommand::Status { socket }, &mut output)
        .await
        .unwrap();

    server.abort();
    daemon.shutdown().await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["response"]["type"], "daemon_status");
}

#[tokio::test]
async fn daemon_stop_command_writes_success_json() {
    let root = unique_temp_dir("styx-cli-daemon-stop");
    let socket = root.join("styx.sock");
    let daemon = DaemonRuntime::start(daemon_config(&root, &socket))
        .await
        .unwrap();
    let server_socket = socket.clone();
    let server_daemon = daemon.clone();
    let server =
        tokio::spawn(async move { serve_daemon_socket(&server_socket, server_daemon).await });
    wait_for_socket(&socket).await;
    let mut output = Vec::new();

    run_daemon_command_once(DaemonCommand::Stop { socket }, &mut output)
        .await
        .unwrap();

    server.abort();
    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["response"]["type"], "daemon_stopped");
}

fn daemon_config(root: &std::path::Path, socket: &std::path::Path) -> DaemonConfig {
    DaemonConfig {
        state_dir: root.join("state"),
        socket_path: socket.to_path_buf(),
        tick_interval: Duration::from_millis(10),
        runtime_config: RuntimeConfig::default(),
    }
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

async fn wait_for_socket(socket: &std::path::Path) {
    for _ in 0..100 {
        if socket.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("socket was not created");
}
