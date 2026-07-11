use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use styx_app::{commands::CommandResponse, ControlCommand, TorrentRuntime};
use styx_dht::{DhtMessage, DhtQuery, DhtResponse, DhtSocket, NodeId};
use styx_runtime::{
    spawn_session_supervisor, AppRuntime, RuntimeConfig, SessionNotice, SessionOwner, ShutdownMode,
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
fn session_owner_is_not_cloneable() {
    static_assertions::assert_not_impl_any!(SessionOwner: Clone);
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
