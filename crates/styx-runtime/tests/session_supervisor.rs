use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use styx_app::{commands::CommandResponse, ControlCommand, TorrentRuntime};
use styx_dht::{DhtMessage, DhtQuery, DhtResponse, DhtSocket, NodeId};
use styx_runtime::{
    spawn_session_supervisor, AppRuntime, DhtOwner, LsdOwner, OwnedTask, RuntimeConfig,
    SessionNotice, SessionOwner, ShutdownMode,
};

#[test]
fn app_runtime_without_session_keeps_synchronous_status_contract() {
    let mut runtime = AppRuntime::new_with_config(RuntimeConfig::default()).unwrap();

    let response = runtime.apply(ControlCommand::Status).unwrap();

    let CommandResponse::Status { snapshot } = response else {
        panic!("expected status response");
    };
    assert!(snapshot.torrents.is_empty());
    assert_eq!(snapshot.totals.torrent_count, 0);
}

#[test]
fn ownership_tokens_are_not_cloneable() {
    static_assertions::assert_not_impl_any!(SessionOwner: Clone);
    static_assertions::assert_not_impl_any!(DhtOwner: Clone);
    static_assertions::assert_not_impl_any!(LsdOwner: Clone);
    static_assertions::assert_not_impl_any!(OwnedTask: Clone);
}

#[test]
fn production_tokio_spawns_are_confined_to_approved_factories() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let approved = [
        "daemon.rs",
        "dht.rs",
        "driver.rs",
        "lsd.rs",
        "peer_io.rs",
        "supervision/supervisor.rs",
    ];
    let mut violations = Vec::new();

    for relative in rust_sources(&root) {
        let source = std::fs::read_to_string(root.join(&relative)).unwrap();
        let production = source.split("#[cfg(test)]").next().unwrap_or(&source);
        if production.contains("tokio::spawn(")
            && !approved.contains(&relative.to_string_lossy().as_ref())
        {
            violations.push(relative);
        }
    }

    assert!(
        violations.is_empty(),
        "production tokio::spawn outside approved factories: {violations:?}"
    );
}

fn rust_sources(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    sources
}

#[tokio::test]
async fn session_routes_dht_bootstrap_as_redacted_notice() {
    let responder = DhtSocket::bind(localhost_ephemeral()).await.unwrap();
    let responder_addr = responder.local_addr().unwrap();
    let config = session_config_with_bootstrap(responder_addr);
    let (client, mut events, owner) = spawn_session_supervisor(config).await.unwrap();

    client.bootstrap_dht().unwrap();
    answer_ping(&responder, NodeId::new([9; 20])).await;

    let notice = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    let report = owner.shutdown(ShutdownMode::Clean).await.unwrap();

    assert_eq!(notice, SessionNotice::DhtBootstrapped { nodes: 1 });
    assert_eq!(report.aborted_count(), 0);
}

#[tokio::test]
async fn dropping_session_client_does_not_detach_shared_workers() {
    let config = RuntimeConfig {
        dht: styx_runtime::DhtRuntimeConfig {
            enabled: false,
            ..styx_runtime::DhtRuntimeConfig::default()
        },
        ..RuntimeConfig::default()
    };
    let (client, _events, owner) = spawn_session_supervisor(config).await.unwrap();
    drop(client);

    let report = owner.shutdown(ShutdownMode::Clean).await.unwrap();

    assert_eq!(report.aborted_count(), 0);
}

#[tokio::test]
async fn enabled_dht_without_bootstrap_nodes_remains_owned_and_shutdown_cleanly() {
    let config = RuntimeConfig {
        dht: styx_runtime::DhtRuntimeConfig {
            enabled: true,
            bind: localhost_ephemeral(),
            bootstrap_nodes: Vec::new(),
            ..styx_runtime::DhtRuntimeConfig::default()
        },
        ..RuntimeConfig::default()
    };
    let (client, _events, owner) = spawn_session_supervisor(config).await.unwrap();

    client.bootstrap_dht().unwrap();
    let report = owner.shutdown(ShutdownMode::Clean).await.unwrap();

    assert_eq!(
        report.exits[&styx_runtime::TaskKind::Dht],
        vec![styx_runtime::TaskExit::Graceful]
    );
    assert!(report.capability_failures.is_empty());
}

fn session_config_with_bootstrap(responder: SocketAddr) -> RuntimeConfig {
    RuntimeConfig {
        dht: styx_runtime::DhtRuntimeConfig {
            enabled: true,
            bind: localhost_ephemeral(),
            bootstrap_nodes: vec![responder],
            query_timeout: Duration::from_millis(100),
            tick_interval: Duration::from_millis(5),
            ..styx_runtime::DhtRuntimeConfig::default()
        },
        ..RuntimeConfig::default()
    }
}

fn localhost_ephemeral() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
}

async fn answer_ping(socket: &DhtSocket, id: NodeId) {
    let event = socket.poll_once().await.unwrap();
    let DhtMessage::Query {
        transaction_id,
        query: DhtQuery::Ping { .. },
    } = event.message
    else {
        panic!("expected ping query");
    };
    socket
        .send_to(
            &DhtMessage::Response {
                transaction_id,
                response: DhtResponse::Ping { id },
            },
            event.source,
        )
        .await
        .unwrap();
}
