use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bytes::Bytes;
use styx_dht::{
    CompactNode, CompactPeer, DhtError, DhtMessage, DhtQuery, DhtResponse, InfoHash, KrpcError,
    NodeAddr, NodeId, TransactionId,
};

#[test]
fn ping_query_round_trips_through_bencode() {
    let message = DhtMessage::Query {
        transaction_id: TransactionId::new(vec![b'a', b'a']).unwrap(),
        query: DhtQuery::Ping {
            id: NodeId::new([1; 20]),
        },
    };

    let encoded = message.encode().unwrap();
    let decoded = DhtMessage::decode(&encoded).unwrap();

    assert_eq!(decoded, message);
}

#[test]
fn find_node_query_round_trips_through_bencode() {
    let message = DhtMessage::Query {
        transaction_id: TransactionId::new(vec![b'f']).unwrap(),
        query: DhtQuery::FindNode {
            id: NodeId::new([1; 20]),
            target: NodeId::new([2; 20]),
        },
    };

    let decoded = DhtMessage::decode(&message.encode().unwrap()).unwrap();

    assert_eq!(decoded, message);
}

#[test]
fn get_peers_query_round_trips_through_bencode() {
    let message = DhtMessage::Query {
        transaction_id: TransactionId::new(vec![b'g']).unwrap(),
        query: DhtQuery::GetPeers {
            id: NodeId::new([1; 20]),
            info_hash: InfoHash::new([3; 20]),
        },
    };

    let decoded = DhtMessage::decode(&message.encode().unwrap()).unwrap();

    assert_eq!(decoded, message);
}

#[test]
fn announce_peer_query_round_trips_through_bencode() {
    let message = DhtMessage::Query {
        transaction_id: TransactionId::new(vec![b'a']).unwrap(),
        query: DhtQuery::AnnouncePeer {
            id: NodeId::new([1; 20]),
            implied_port: true,
            info_hash: InfoHash::new([3; 20]),
            port: 6881,
            token: Bytes::from_static(b"tok"),
        },
    };

    let decoded = DhtMessage::decode(&message.encode().unwrap()).unwrap();

    assert_eq!(decoded, message);
}

#[test]
fn get_peers_response_with_peers_round_trips_through_bencode() {
    let response = DhtMessage::Response {
        transaction_id: TransactionId::new(vec![b'g', b'p']).unwrap(),
        response: DhtResponse::GetPeers {
            id: NodeId::new([2; 20]),
            token: Bytes::from_static(b"tok"),
            values: vec![CompactPeer::new(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                6881,
            ))],
            nodes: Vec::new(),
        },
    };

    let decoded = DhtMessage::decode(&response.encode().unwrap()).unwrap();

    assert_eq!(decoded, response);
}

#[test]
fn find_node_response_with_compact_nodes_round_trips_through_bencode() {
    let node = CompactNode {
        id: NodeId::new([3; 20]),
        addr: NodeAddr::new(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            51413,
        )),
    };
    let response = DhtMessage::Response {
        transaction_id: TransactionId::new(vec![b'f', b'n']).unwrap(),
        response: DhtResponse::FindNode {
            id: NodeId::new([4; 20]),
            nodes: vec![node],
        },
    };

    let decoded = DhtMessage::decode(&response.encode().unwrap()).unwrap();

    assert_eq!(decoded, response);
}

#[test]
fn krpc_error_round_trips_through_bencode() {
    let message = DhtMessage::Error {
        transaction_id: TransactionId::new(vec![b'e']).unwrap(),
        error: KrpcError {
            code: 203,
            message: "Protocol Error".to_owned(),
        },
    };

    let decoded = DhtMessage::decode(&message.encode().unwrap()).unwrap();

    assert_eq!(decoded, message);
}

#[test]
fn decode_rejects_unknown_message_kind() {
    let err = DhtMessage::decode(b"d1:t2:aa1:y1:xe").unwrap_err();

    assert_eq!(err, DhtError::InvalidMessage("unknown KRPC message kind"));
}

#[test]
fn decode_rejects_missing_transaction_id() {
    let err = DhtMessage::decode(b"d1:q4:ping1:y1:qe").unwrap_err();

    assert_eq!(err, DhtError::MissingField("t"));
}
