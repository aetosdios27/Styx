use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use styx_dht::{CompactNode, CompactPeer, DhtError, InfoHash, NodeAddr, NodeId, TransactionId};

#[test]
fn node_id_rejects_wrong_length() {
    let err = NodeId::try_from(&[1_u8; 19][..]).unwrap_err();

    assert_eq!(
        err,
        DhtError::InvalidLength {
            expected: 20,
            actual: 19
        }
    );
}

#[test]
fn compact_node_round_trips_twenty_byte_id_ipv4_and_port() {
    let id = NodeId::new([7; 20]);
    let addr = NodeAddr::new(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        6881,
    ));
    let compact = CompactNode { id, addr };

    let encoded = compact.encode_ipv4().unwrap();
    let decoded = CompactNode::decode_ipv4(&encoded).unwrap();

    assert_eq!(decoded, compact);
}

#[test]
fn compact_peer_round_trips_ipv4_and_port() {
    let peer = CompactPeer::new(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
        51413,
    ));

    let encoded = peer.encode_ipv4().unwrap();
    let decoded = CompactPeer::decode_ipv4(&encoded).unwrap();

    assert_eq!(decoded, peer);
}

#[test]
fn node_distance_orders_ids_by_xor_distance() {
    let origin = NodeId::new([0; 20]);
    let near = NodeId::new([0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let far = NodeId::new([
        0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);

    assert!(origin.distance(&near) < origin.distance(&far));
}

#[test]
fn transaction_id_is_bounded_to_four_bytes() {
    let err = TransactionId::new(vec![1, 2, 3, 4, 5]).unwrap_err();

    assert_eq!(err, DhtError::TransactionIdTooLong { len: 5, max: 4 });
}

#[test]
fn info_hash_accepts_exact_twenty_bytes() {
    let hash = InfoHash::try_from(&[9_u8; 20][..]).unwrap();

    assert_eq!(hash.as_bytes(), &[9_u8; 20]);
}
