use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use styx_dht::{DhtMessage, DhtQuery, DhtResponse, DhtSocket, InfoHash, NodeId};
use styx_proto::InfoHashV1;
use styx_runtime::{
    spawn_dht_worker, DhtCommand, DhtRuntimeConfig, DhtRuntimeEvent, RuntimeConfig, TorrentId,
};
use tokio::time::timeout;

#[test]
fn runtime_config_default_has_dht_enabled_with_safe_caps() {
    let config = RuntimeConfig::default();

    assert!(config.dht.enabled);
    assert_eq!(config.dht.metadata_size_limit, 8 * 1024 * 1024);
    assert_eq!(config.dht.metadata_request_limit, 512);
}

#[tokio::test]
async fn dht_worker_bootstrap_emits_bootstrapped_after_local_responder() {
    let remote = DhtSocket::bind(localhost_ephemeral()).await.unwrap();
    let remote_addr = remote.local_addr().unwrap();
    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
    let worker = spawn_dht_worker(test_dht_config(remote_addr), events_tx)
        .await
        .unwrap();

    worker.send(DhtCommand::Bootstrap).unwrap();
    respond_to_ping(remote, NodeId::new([9; 20])).await;

    let event = timeout(Duration::from_secs(1), events_rx.recv())
        .await
        .unwrap()
        .unwrap();
    worker.shutdown().await.unwrap();

    assert!(matches!(event, DhtRuntimeEvent::Bootstrapped { nodes: 1 }));
}

#[tokio::test]
async fn dht_worker_shutdown_drops_socket_without_hanging() {
    let (events_tx, _events_rx) = tokio::sync::mpsc::unbounded_channel();
    let config = DhtRuntimeConfig {
        bootstrap_nodes: Vec::new(),
        ..test_dht_config(localhost_ephemeral())
    };
    let worker = spawn_dht_worker(config, events_tx).await.unwrap();

    timeout(Duration::from_secs(1), worker.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn dht_worker_reports_lookup_exhausted_after_timeout() {
    let remote = DhtSocket::bind(localhost_ephemeral()).await.unwrap();
    let remote_addr = remote.local_addr().unwrap();
    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
    let worker = spawn_dht_worker(test_dht_config(remote_addr), events_tx)
        .await
        .unwrap();
    let info_hash = InfoHash::new([42; 20]);
    let torrent = TorrentId::new(InfoHashV1::new([42; 20]));

    worker.send(DhtCommand::Bootstrap).unwrap();
    respond_to_ping(remote, NodeId::new([10; 20])).await;
    wait_for_bootstrap(&mut events_rx).await;
    worker
        .send(DhtCommand::GetPeers { torrent, info_hash })
        .unwrap();

    let event = timeout(
        Duration::from_secs(1),
        wait_for_lookup_exhausted(&mut events_rx),
    )
    .await
    .unwrap();
    worker.shutdown().await.unwrap();

    assert_eq!(event, DhtRuntimeEvent::LookupExhausted { torrent });
}

fn localhost_ephemeral() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
}

fn test_dht_config(remote_addr: SocketAddr) -> DhtRuntimeConfig {
    DhtRuntimeConfig {
        enabled: true,
        bind: localhost_ephemeral(),
        bootstrap_nodes: vec![remote_addr],
        query_timeout: Duration::from_millis(30),
        metadata_size_limit: 8 * 1024 * 1024,
        metadata_request_limit: 512,
        tick_interval: Duration::from_millis(5),
    }
}

async fn respond_to_ping(socket: DhtSocket, id: NodeId) {
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

async fn wait_for_bootstrap(events_rx: &mut tokio::sync::mpsc::UnboundedReceiver<DhtRuntimeEvent>) {
    loop {
        let event = timeout(Duration::from_secs(1), events_rx.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(event, DhtRuntimeEvent::Bootstrapped { .. }) {
            return;
        }
    }
}

async fn wait_for_lookup_exhausted(
    events_rx: &mut tokio::sync::mpsc::UnboundedReceiver<DhtRuntimeEvent>,
) -> DhtRuntimeEvent {
    loop {
        let event = events_rx.recv().await.unwrap();
        if matches!(event, DhtRuntimeEvent::LookupExhausted { .. }) {
            return event;
        }
    }
}
