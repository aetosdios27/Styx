use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bytes::Bytes;
use styx_dht::{
    CompactPeer, DhtConfig, DhtEvent, DhtMessage, DhtQuery, DhtResponse, DhtRuntime, DhtSocket,
    DhtSocketRuntime, InfoHash, NodeId, TokenManager, TransactionId,
};

#[tokio::test]
#[ignore = "requires UDP socket permissions in the test environment"]
async fn udp_socket_exchanges_krpc_message() {
    let first = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let second = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let message = DhtMessage::Query {
        transaction_id: TransactionId::new(vec![b'a']).unwrap(),
        query: DhtQuery::Ping {
            id: NodeId::new([1; 20]),
        },
    };

    first
        .send_to(&message, second.local_addr().unwrap())
        .await
        .unwrap();
    let event = second.poll_once().await.unwrap();

    assert_eq!(event.message, message);
    assert_eq!(event.source, first.local_addr().unwrap());
}

#[tokio::test]
#[ignore = "requires UDP socket permissions in the test environment"]
async fn socket_runtime_responds_to_inbound_ping_query() {
    let server_socket = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let client_socket = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let server_addr = server_socket.local_addr().unwrap();
    let server_id = NodeId::new([9; 20]);
    let runtime = DhtRuntime::new(
        server_id,
        TokenManager::with_secrets(
            Bytes::from_static(b"socket-runtime-current"),
            Bytes::from_static(b"socket-runtime-previous"),
        ),
        DhtConfig::default(),
    )
    .unwrap();
    let mut socket_runtime = DhtSocketRuntime::new(server_socket, runtime);
    let transaction_id = TransactionId::new(vec![b's']).unwrap();

    client_socket
        .send_to(
            &DhtMessage::Query {
                transaction_id: transaction_id.clone(),
                query: DhtQuery::Ping {
                    id: NodeId::new([8; 20]),
                },
            },
            server_addr,
        )
        .await
        .unwrap();

    socket_runtime.step_once().await.unwrap();
    let response = client_socket.poll_once().await.unwrap();

    assert_eq!(
        response.message,
        DhtMessage::Response {
            transaction_id,
            response: DhtResponse::Ping { id: server_id },
        }
    );
}

#[tokio::test]
#[ignore = "requires UDP socket permissions in the test environment"]
async fn socket_runtime_get_peers_once_discovers_peer() {
    let remote_socket = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let client_socket = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let remote_addr = remote_socket.local_addr().unwrap();
    let remote_id = NodeId::new([21; 20]);
    let info_hash = InfoHash::new([22; 20]);
    let peer = CompactPeer::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51413));
    let remote = tokio::spawn(async move {
        let request = remote_socket.poll_once().await.unwrap();
        let DhtMessage::Query { transaction_id, .. } = request.message else {
            panic!("expected get_peers query");
        };
        remote_socket
            .send_to(
                &DhtMessage::Response {
                    transaction_id,
                    response: DhtResponse::GetPeers {
                        id: remote_id,
                        token: Bytes::from_static(b"tok"),
                        values: vec![peer],
                        nodes: Vec::new(),
                        nodes6: Vec::new(),
                        external_ip: None,
                    },
                },
                request.source,
            )
            .await
            .unwrap();
    });
    let mut config = DhtConfig::default();
    config.add_bootstrap_node(remote_addr);
    let mut client = DhtSocketRuntime::new(
        client_socket,
        runtime_with_config(NodeId::new([20; 20]), config),
    );
    client
        .runtime_mut()
        .node_mut()
        .routing_mut()
        .insert(remote_id, styx_dht::NodeAddr::new(remote_addr))
        .unwrap();

    let action = client.get_peers_once(info_hash).await.unwrap();
    remote.await.unwrap();

    assert_eq!(
        action.event,
        Some(DhtEvent::PeersDiscovered {
            info_hash,
            peers: vec![peer],
        })
    );
}

#[tokio::test]
#[ignore = "requires UDP socket permissions in the test environment"]
async fn socket_runtime_announce_peer_once_sends_token_back_to_responder() {
    let remote_socket = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let client_socket = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let remote_addr = remote_socket.local_addr().unwrap();
    let remote_id = NodeId::new([31; 20]);
    let info_hash = InfoHash::new([32; 20]);
    let remote = tokio::spawn(async move {
        let first = remote_socket.poll_once().await.unwrap();
        let DhtMessage::Query {
            transaction_id: first_tx,
            ..
        } = first.message
        else {
            panic!("expected get_peers query");
        };
        remote_socket
            .send_to(
                &DhtMessage::Response {
                    transaction_id: first_tx,
                    response: DhtResponse::GetPeers {
                        id: remote_id,
                        token: Bytes::from_static(b"tok"),
                        values: Vec::new(),
                        nodes: Vec::new(),
                        nodes6: Vec::new(),
                        external_ip: None,
                    },
                },
                first.source,
            )
            .await
            .unwrap();
        let second = remote_socket.poll_once().await.unwrap();
        let DhtMessage::Query {
            query: DhtQuery::AnnouncePeer { token, .. },
            ..
        } = second.message
        else {
            panic!("expected announce_peer query");
        };
        assert_eq!(token, Bytes::from_static(b"tok"));
    });
    let mut config = DhtConfig::default();
    config.add_bootstrap_node(remote_addr);
    let mut client = DhtSocketRuntime::new(
        client_socket,
        runtime_with_config(NodeId::new([30; 20]), config),
    );
    client
        .runtime_mut()
        .node_mut()
        .routing_mut()
        .insert(remote_id, styx_dht::NodeAddr::new(remote_addr))
        .unwrap();

    client.get_peers_once(info_hash).await.unwrap();
    client
        .announce_peer_once(info_hash, 51413, false)
        .await
        .unwrap();
    remote.await.unwrap();
}

fn runtime_with_config(id: NodeId, config: DhtConfig) -> DhtRuntime {
    DhtRuntime::new(
        id,
        TokenManager::with_secrets(
            Bytes::from_static(b"socket-runtime-current"),
            Bytes::from_static(b"socket-runtime-previous"),
        ),
        config,
    )
    .unwrap()
}
