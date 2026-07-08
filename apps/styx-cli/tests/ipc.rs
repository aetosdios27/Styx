use styx_app::{CommandEnvelope, CommandResponseEnvelope, ControlCommand};
#[cfg(unix)]
use styx_cli::ipc::serve_daemon_socket;
use styx_cli::ipc::{decode_command, encode_command, encode_response, send_unix_command};

#[cfg(unix)]
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(unix)]
use styx_runtime::{DaemonConfig, DaemonRuntime, RuntimeConfig};
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[test]
fn command_codec_rejects_trailing_json() {
    let err = decode_command(br#"{"type":"status"}{"type":"status"}"#).unwrap_err();

    assert!(err.to_string().contains("trailing"));
}

#[test]
fn command_codec_round_trips_one_command_per_line() {
    let encoded = encode_command(&ControlCommand::Status).unwrap();

    let decoded = decode_command(&encoded).unwrap();

    assert_eq!(decoded, ControlCommand::Status);
}

#[test]
fn response_envelope_serializes_failure() {
    let encoded = encode_response(&CommandResponseEnvelope::err("bad command")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();

    assert_eq!(value["ok"], false);
}

#[test]
fn command_envelope_defaults_to_current_protocol_version() {
    let envelope = CommandEnvelope::new(ControlCommand::Status);

    assert_eq!(envelope.version, 1);
}

#[cfg(unix)]
#[tokio::test]
async fn ipc_server_routes_status_to_daemon_handle() {
    let root = unique_temp_dir("styx-cli-ipc-status");
    let socket = root.join("styx.sock");
    let daemon = DaemonRuntime::start(daemon_config(&root, &socket))
        .await
        .unwrap();
    let server_socket = socket.clone();
    let server_daemon = daemon.clone();
    let server =
        tokio::spawn(async move { serve_daemon_socket(&server_socket, server_daemon).await });
    wait_for_socket(&socket).await;

    let response = send_unix_command(&socket, &ControlCommand::Status)
        .await
        .unwrap();

    server.abort();
    daemon.shutdown().await.unwrap();
    assert!(response.ok);
}

#[cfg(unix)]
#[tokio::test]
async fn ipc_server_returns_error_for_malformed_json_and_keeps_running() {
    let root = unique_temp_dir("styx-cli-ipc-malformed");
    let socket = root.join("styx.sock");
    let daemon = DaemonRuntime::start(daemon_config(&root, &socket))
        .await
        .unwrap();
    let server_socket = socket.clone();
    let server_daemon = daemon.clone();
    let server =
        tokio::spawn(async move { serve_daemon_socket(&server_socket, server_daemon).await });
    wait_for_socket(&socket).await;

    let malformed = send_raw_line(&socket, b"{bad-json\n").await;
    let status = send_unix_command(&socket, &ControlCommand::Status)
        .await
        .unwrap();

    server.abort();
    daemon.shutdown().await.unwrap();
    assert!(!malformed.ok);
    assert!(status.ok);
}

#[cfg(unix)]
fn daemon_config(root: &std::path::Path, socket: &std::path::Path) -> DaemonConfig {
    DaemonConfig {
        state_dir: root.join("state"),
        socket_path: socket.to_path_buf(),
        tick_interval: Duration::from_millis(10),
        runtime_config: RuntimeConfig::default(),
    }
}

#[cfg(unix)]
fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[cfg(unix)]
async fn wait_for_socket(socket: &std::path::Path) {
    for _ in 0..100 {
        if socket.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("socket was not created");
}

#[cfg(unix)]
async fn send_raw_line(socket: &std::path::Path, line: &[u8]) -> CommandResponseEnvelope {
    let mut stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    stream.write_all(line).await.unwrap();
    stream.shutdown().await.unwrap();
    let mut reader = BufReader::new(stream);
    let mut response = Vec::new();
    reader.read_until(b'\n', &mut response).await.unwrap();
    serde_json::from_slice(&response).unwrap()
}
