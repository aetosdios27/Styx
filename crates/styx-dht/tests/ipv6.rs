use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use styx_dht::{CompactNode, CompactPeer, NodeAddr, NodeId};

#[test]
fn compact_peer_round_trips_ipv6_and_port() {
    let peer = CompactPeer::new(SocketAddr::new(
        IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
        51413,
    ));

    let encoded = peer.encode_ipv6().unwrap();
    let decoded = CompactPeer::decode_ipv6(&encoded).unwrap();

    assert_eq!(decoded, peer);
}

#[test]
fn compact_node_round_trips_twenty_byte_id_ipv6_and_port() {
    let node = CompactNode {
        id: NodeId::new([42; 20]),
        addr: NodeAddr::new(SocketAddr::new(
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2)),
            6881,
        )),
    };

    let encoded = node.encode_ipv6().unwrap();
    let decoded = CompactNode::decode_ipv6(&encoded).unwrap();

    assert_eq!(decoded, node);
}
