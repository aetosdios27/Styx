use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bytes::Bytes;
use styx_dht::{
    CompactNode, CompactPeer, DhtMessage, DhtNode, DhtQuery, DhtResponse, InfoHash, Lookup,
    LookupKind, NodeAddr, NodeId, TokenManager, TransactionId,
};

#[test]
fn simulated_get_peers_lookup_discovers_peer_through_known_node() {
    let info_hash = InfoHash::new([7; 20]);
    let mut lookup = Lookup::new(LookupKind::GetPeers { info_hash }, 1, vec![compact_node(2)]);
    let mut remote = DhtNode::new(
        node_id(2),
        TokenManager::with_secrets(Bytes::from_static(b"a"), Bytes::from_static(b"b")),
    );
    let peer = CompactPeer::new(socket(99));
    remote.announce_local_peer(info_hash, peer).unwrap();

    let target = lookup.next_query_batch().remove(0);
    let response = remote
        .handle_message(
            DhtMessage::Query {
                transaction_id: TransactionId::new(vec![b'g']).unwrap(),
                query: DhtQuery::GetPeers {
                    id: node_id(1),
                    info_hash,
                },
            },
            node_addr(1),
        )
        .unwrap();

    let DhtMessage::Response {
        response: DhtResponse::GetPeers { values, .. },
        ..
    } = response
    else {
        panic!("expected get_peers response");
    };
    lookup.on_peers(target.id, values);

    assert!(lookup.is_complete());
    assert_eq!(lookup.peers(), vec![peer]);
}

fn compact_node(first_byte: u8) -> CompactNode {
    CompactNode {
        id: node_id(first_byte),
        addr: node_addr(first_byte),
    }
}

fn node_id(first_byte: u8) -> NodeId {
    let mut bytes = [0_u8; 20];
    bytes[0] = first_byte;
    NodeId::new(bytes)
}

fn node_addr(last_octet: u8) -> NodeAddr {
    NodeAddr::new(socket(last_octet))
}

fn socket(last_octet: u8) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, last_octet)), 6881)
}
