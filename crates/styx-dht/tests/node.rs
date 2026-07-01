use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bytes::Bytes;
use styx_dht::{
    CompactPeer, DhtError, DhtMessage, DhtNode, DhtQuery, DhtResponse, InfoHash, NodeAddr, NodeId,
    TokenManager, TransactionId,
};

#[test]
fn ping_query_returns_local_node_id() {
    let local = NodeId::new([9; 20]);
    let mut node = DhtNode::new(
        local,
        TokenManager::with_secrets(Bytes::from_static(b"a"), Bytes::from_static(b"b")),
    );
    let response = node
        .handle_message(ping(NodeId::new([1; 20])), source(1))
        .unwrap();

    assert_eq!(
        response,
        DhtMessage::Response {
            transaction_id: TransactionId::new(vec![b'a']).unwrap(),
            response: DhtResponse::Ping { id: local },
        }
    );
}

#[test]
fn get_peers_returns_known_peers_and_token() {
    let local = NodeId::new([9; 20]);
    let hash = InfoHash::new([3; 20]);
    let mut node = DhtNode::new(
        local,
        TokenManager::with_secrets(Bytes::from_static(b"a"), Bytes::from_static(b"b")),
    );
    let peer = CompactPeer::new(source(7).socket_addr());
    node.announce_local_peer(hash, peer).unwrap();

    let response = node
        .handle_message(
            DhtMessage::Query {
                transaction_id: TransactionId::new(vec![b'g']).unwrap(),
                query: DhtQuery::GetPeers {
                    id: NodeId::new([1; 20]),
                    info_hash: hash,
                    want: Vec::new(),
                },
            },
            source(1),
        )
        .unwrap();

    let DhtMessage::Response {
        response:
            DhtResponse::GetPeers {
                id,
                token,
                values,
                nodes,
                ..
            },
        ..
    } = response
    else {
        panic!("expected get_peers response");
    };
    assert_eq!(id, local);
    assert!(!token.is_empty());
    assert_eq!(values, vec![peer]);
    assert!(nodes.is_empty());
}

#[test]
fn get_peers_without_known_peers_returns_closest_nodes_and_token() {
    let local = NodeId::new([0; 20]);
    let hash = InfoHash::new([3; 20]);
    let mut node = DhtNode::new(
        local,
        TokenManager::with_secrets(Bytes::from_static(b"a"), Bytes::from_static(b"b")),
    );
    node.routing_mut().insert(node_id(2), source(2)).unwrap();

    let response = node
        .handle_message(
            DhtMessage::Query {
                transaction_id: TransactionId::new(vec![b'g']).unwrap(),
                query: DhtQuery::GetPeers {
                    id: node_id(1),
                    info_hash: hash,
                    want: Vec::new(),
                },
            },
            source(1),
        )
        .unwrap();

    let DhtMessage::Response {
        response: DhtResponse::GetPeers { values, nodes, .. },
        ..
    } = response
    else {
        panic!("expected get_peers response");
    };
    assert!(values.is_empty());
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].id, node_id(2));
}

#[test]
fn announce_peer_requires_valid_token_for_source_ip() {
    let local = NodeId::new([9; 20]);
    let hash = InfoHash::new([3; 20]);
    let mut node = DhtNode::new(
        local,
        TokenManager::with_secrets(Bytes::from_static(b"a"), Bytes::from_static(b"b")),
    );
    let err = node
        .handle_message(
            DhtMessage::Query {
                transaction_id: TransactionId::new(vec![b'a']).unwrap(),
                query: DhtQuery::AnnouncePeer {
                    id: node_id(1),
                    implied_port: false,
                    info_hash: hash,
                    port: 6881,
                    token: Bytes::from_static(b"wrong"),
                },
            },
            source(1),
        )
        .unwrap_err();

    assert_eq!(err, DhtError::InvalidToken);
}

#[test]
fn announce_peer_with_valid_token_stores_peer() {
    let local = NodeId::new([9; 20]);
    let hash = InfoHash::new([3; 20]);
    let source = source(1);
    let manager = TokenManager::with_secrets(Bytes::from_static(b"a"), Bytes::from_static(b"b"));
    let token = manager.issue(source.socket_addr().ip());
    let mut node = DhtNode::new(local, manager);

    node.handle_message(
        DhtMessage::Query {
            transaction_id: TransactionId::new(vec![b'a']).unwrap(),
            query: DhtQuery::AnnouncePeer {
                id: node_id(1),
                implied_port: true,
                info_hash: hash,
                port: 1,
                token,
            },
        },
        source,
    )
    .unwrap();

    assert_eq!(
        node.peers(hash),
        vec![CompactPeer::new(source.socket_addr())]
    );
}

fn ping(id: NodeId) -> DhtMessage {
    DhtMessage::Query {
        transaction_id: TransactionId::new(vec![b'a']).unwrap(),
        query: DhtQuery::Ping { id },
    }
}

fn node_id(first_byte: u8) -> NodeId {
    let mut bytes = [0_u8; 20];
    bytes[0] = first_byte;
    NodeId::new(bytes)
}

fn source(last_octet: u8) -> NodeAddr {
    NodeAddr::new(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, last_octet)),
        6881,
    ))
}
