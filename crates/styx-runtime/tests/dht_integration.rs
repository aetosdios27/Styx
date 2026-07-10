use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use bytes::Bytes;
use styx_dht::{CompactPeer, DhtMessage, DhtQuery, DhtResponse, DhtSocket, InfoHash, NodeId};
use styx_proto::InfoHashV1;
use styx_runtime::{
    spawn_dht_worker, DhtCommand, DhtOwner, DhtRuntimeConfig, DhtRuntimeEvent, RuntimeConfig,
    RuntimeError, ShutdownMode, TaskExit, TaskKind, TaskRegistry, TorrentId,
};
use tokio::time::timeout;

#[test]
fn runtime_config_default_has_dht_enabled_with_safe_caps() {
    let config = RuntimeConfig::default();

    assert!(config.dht.enabled);
    assert_eq!(config.dht.metadata_size_limit, 8 * 1024 * 1024);
    assert_eq!(config.dht.metadata_request_limit, 512);
    assert_eq!(config.dht.command_capacity, 256);
}

#[test]
fn dht_owner_is_not_cloneable() {
    static_assertions::assert_not_impl_any!(DhtOwner: Clone);
}

#[test]
fn dht_config_rejects_zero_command_capacity() {
    let error = DhtRuntimeConfig {
        command_capacity: 0,
        ..DhtRuntimeConfig::default()
    }
    .validate()
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        RuntimeError::InvalidConfig("dht command capacity must be greater than zero").to_string()
    );
}

#[tokio::test]
async fn dht_worker_bootstrap_emits_bootstrapped_after_local_responder() {
    let remote = DhtSocket::bind(localhost_ephemeral()).await.unwrap();
    let remote_addr = remote.local_addr().unwrap();
    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (client, owner) = spawn_dht_worker(test_dht_config(remote_addr), events_tx)
        .await
        .unwrap();

    client.try_send(DhtCommand::Bootstrap).unwrap();
    respond_to_ping(&remote, NodeId::new([9; 20])).await;

    let event = timeout(Duration::from_secs(1), events_rx.recv())
        .await
        .unwrap()
        .unwrap();
    owner.shutdown().await.unwrap();

    assert!(matches!(event, DhtRuntimeEvent::Bootstrapped { nodes: 1 }));
}

#[tokio::test]
async fn dht_worker_shutdown_drops_socket_without_hanging() {
    let (events_tx, _events_rx) = tokio::sync::mpsc::unbounded_channel();
    let config = DhtRuntimeConfig {
        bootstrap_nodes: Vec::new(),
        ..test_dht_config(localhost_ephemeral())
    };
    let (client, owner) = spawn_dht_worker(config, events_tx).await.unwrap();
    drop(client);

    timeout(Duration::from_secs(1), owner.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn dht_owner_into_task_preserves_cooperative_registry_shutdown() {
    let (events_tx, _events_rx) = tokio::sync::mpsc::unbounded_channel();
    let config = DhtRuntimeConfig {
        bootstrap_nodes: Vec::new(),
        ..test_dht_config(localhost_ephemeral())
    };
    let (client, owner) = spawn_dht_worker(config, events_tx).await.unwrap();
    let mut registry = TaskRegistry::default();
    registry.register(owner.into_task());
    tokio::task::yield_now().await;
    client.try_send(DhtCommand::Bootstrap).unwrap();

    let exits = registry
        .shutdown(
            ShutdownMode::Clean,
            Duration::from_millis(50),
            Duration::from_millis(50),
        )
        .await;

    assert_eq!(exits[&TaskKind::Dht], vec![TaskExit::Graceful]);
}

#[tokio::test]
async fn dropping_dht_owner_aborts_live_worker_and_closes_clients() {
    let (events_tx, _events_rx) = tokio::sync::mpsc::unbounded_channel();
    let config = DhtRuntimeConfig {
        bootstrap_nodes: Vec::new(),
        ..test_dht_config(localhost_ephemeral())
    };
    let (client, owner) = spawn_dht_worker(config, events_tx).await.unwrap();
    drop(owner);

    timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                client.try_send(DhtCommand::Bootstrap),
                Err(RuntimeError::Cancelled)
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner drop must abort the worker and close its command receiver");
}

#[tokio::test]
async fn dht_worker_reports_lookup_exhausted_after_timeout() {
    let remote = DhtSocket::bind(localhost_ephemeral()).await.unwrap();
    let remote_addr = remote.local_addr().unwrap();
    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (client, owner) = spawn_dht_worker(test_dht_config(remote_addr), events_tx)
        .await
        .unwrap();
    let info_hash = InfoHash::new([42; 20]);
    let torrent = TorrentId::new(InfoHashV1::new([42; 20]));

    client.try_send(DhtCommand::Bootstrap).unwrap();
    respond_to_ping(&remote, NodeId::new([10; 20])).await;
    wait_for_bootstrap(&mut events_rx).await;
    client
        .try_send(DhtCommand::GetPeers { torrent, info_hash })
        .unwrap();

    let event = timeout(
        Duration::from_secs(1),
        wait_for_lookup_exhausted(&mut events_rx),
    )
    .await
    .unwrap();
    owner.shutdown().await.unwrap();

    assert_eq!(event, DhtRuntimeEvent::LookupExhausted { torrent });
}

#[tokio::test]
async fn dht_announce_uses_prior_token_and_configured_port() {
    let remote = DhtSocket::bind(localhost_ephemeral()).await.unwrap();
    let remote_addr = remote.local_addr().unwrap();
    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (client, owner) = spawn_dht_worker(test_dht_config(remote_addr), events_tx)
        .await
        .unwrap();
    let info_hash = InfoHash::new([51; 20]);
    let torrent = TorrentId::new(InfoHashV1::new([51; 20]));

    client.try_send(DhtCommand::Bootstrap).unwrap();
    respond_to_ping(&remote, NodeId::new([11; 20])).await;
    wait_for_bootstrap(&mut events_rx).await;
    client
        .try_send(DhtCommand::GetPeers { torrent, info_hash })
        .unwrap();

    let get_peers = remote.poll_once().await.unwrap();
    let DhtMessage::Query {
        transaction_id,
        query: DhtQuery::GetPeers { .. },
    } = get_peers.message
    else {
        panic!("expected get_peers query");
    };
    remote
        .send_to(
            &DhtMessage::Response {
                transaction_id,
                response: DhtResponse::GetPeers {
                    id: NodeId::new([11; 20]),
                    token: Bytes::from_static(b"announce-token"),
                    values: vec![CompactPeer::new("127.0.0.1:7000".parse().unwrap())],
                    nodes: Vec::new(),
                    nodes6: Vec::new(),
                    external_ip: None,
                },
            },
            get_peers.source,
        )
        .await
        .unwrap();
    loop {
        if matches!(
            events_rx.recv().await,
            Some(DhtRuntimeEvent::PeersDiscovered { .. })
        ) {
            break;
        }
    }

    client
        .try_send(DhtCommand::AnnouncePeer {
            torrent,
            info_hash,
            port: 6881,
            implied_port: false,
        })
        .unwrap();
    let announce = remote.poll_once().await.unwrap();
    let DhtMessage::Query {
        query:
            DhtQuery::AnnouncePeer {
                port,
                implied_port,
                token,
                ..
            },
        ..
    } = announce.message
    else {
        panic!("expected announce_peer query");
    };

    owner.shutdown().await.unwrap();
    assert_eq!(port, 6881);
    assert!(!implied_port);
    assert_eq!(token, Bytes::from_static(b"announce-token"));
}

#[tokio::test]
async fn dht_announce_does_not_run_without_token() {
    let remote = DhtSocket::bind(localhost_ephemeral()).await.unwrap();
    let remote_addr = remote.local_addr().unwrap();
    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (client, owner) = spawn_dht_worker(test_dht_config(remote_addr), events_tx)
        .await
        .unwrap();
    let info_hash = InfoHash::new([52; 20]);
    let torrent = TorrentId::new(InfoHashV1::new([52; 20]));

    client.try_send(DhtCommand::Bootstrap).unwrap();
    respond_to_ping(&remote, NodeId::new([12; 20])).await;
    wait_for_bootstrap(&mut events_rx).await;
    client
        .try_send(DhtCommand::AnnouncePeer {
            torrent,
            info_hash,
            port: 6881,
            implied_port: false,
        })
        .unwrap();

    let event = timeout(Duration::from_secs(1), events_rx.recv())
        .await
        .unwrap()
        .unwrap();
    owner.shutdown().await.unwrap();
    assert!(matches!(event, DhtRuntimeEvent::Failed { .. }));
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
        command_capacity: 256,
        metadata_size_limit: 8 * 1024 * 1024,
        metadata_request_limit: 512,
        tick_interval: Duration::from_millis(5),
    }
}

async fn respond_to_ping(socket: &DhtSocket, id: NodeId) {
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
