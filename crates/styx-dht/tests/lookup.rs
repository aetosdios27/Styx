use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use styx_dht::{CompactNode, CompactPeer, InfoHash, Lookup, LookupKind, NodeAddr, NodeId};

#[test]
fn lookup_queries_alpha_closest_unqueried_nodes() {
    let target = LookupKind::FindNode {
        target: node_id(10),
    };
    let mut lookup = Lookup::new(
        target,
        2,
        vec![compact_node(9), compact_node(1), compact_node(8)],
    );

    let batch = lookup.next_query_batch();

    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].id, node_id(8));
    assert_eq!(batch[1].id, node_id(9));
}

#[test]
fn lookup_adds_closer_nodes_from_response() {
    let mut lookup = Lookup::new(
        LookupKind::FindNode {
            target: node_id(10),
        },
        1,
        vec![compact_node(1)],
    );
    let queried = lookup.next_query_batch().remove(0);

    lookup.on_nodes(queried.id, vec![compact_node(10), compact_node(9)]);
    let next = lookup.next_query_batch();

    assert_eq!(next[0].id, node_id(10));
}

#[test]
fn get_peers_lookup_finishes_when_peers_arrive() {
    let peer = CompactPeer::new(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        6881,
    ));
    let mut lookup = Lookup::new(
        LookupKind::GetPeers {
            info_hash: InfoHash::new([3; 20]),
        },
        1,
        vec![compact_node(1)],
    );
    let queried = lookup.next_query_batch().remove(0);

    lookup.on_peers(queried.id, vec![peer]);

    assert!(lookup.is_complete());
    assert_eq!(lookup.peers(), vec![peer]);
}

#[test]
fn lookup_finishes_when_all_candidates_are_queried() {
    let mut lookup = Lookup::new(
        LookupKind::FindNode {
            target: node_id(10),
        },
        1,
        vec![compact_node(1)],
    );
    let queried = lookup.next_query_batch().remove(0);

    lookup.on_nodes(queried.id, Vec::new());

    assert!(lookup.is_complete());
}

fn compact_node(first_byte: u8) -> CompactNode {
    CompactNode {
        id: node_id(first_byte),
        addr: NodeAddr::new(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, first_byte)),
            6881,
        )),
    }
}

fn node_id(first_byte: u8) -> NodeId {
    let mut bytes = [0_u8; 20];
    bytes[0] = first_byte;
    NodeId::new(bytes)
}
