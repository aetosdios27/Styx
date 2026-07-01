use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use styx_dht::{DhtError, NodeAddr, NodeId, NodeStatus, RoutingTable, K_BUCKET_SIZE};

#[test]
fn routing_table_accepts_up_to_k_nodes_in_one_bucket() {
    let local = NodeId::new([0; 20]);
    let mut table = RoutingTable::new(local);

    for value in 1..=K_BUCKET_SIZE as u8 {
        table.insert(node_id(value), addr(value)).unwrap();
    }

    assert_eq!(table.len(), K_BUCKET_SIZE);
}

#[test]
fn full_bucket_not_containing_local_node_rejects_extra_node() {
    let local = NodeId::new([0; 20]);
    let mut table = RoutingTable::new(local);

    for value in 0x80..0x88 {
        table.insert(node_id(value), addr(value)).unwrap();
    }
    let err = table.insert(node_id(0x88), addr(0x88)).unwrap_err();

    assert_eq!(err, DhtError::BucketFull);
}

#[test]
fn full_bucket_containing_local_node_splits_to_accept_extra_node() {
    let local = NodeId::new([0; 20]);
    let mut table = RoutingTable::new(local);

    for value in 1..=8 {
        table.insert(node_id(value), addr(value)).unwrap();
    }
    table.insert(node_id(9), addr(9)).unwrap();

    assert_eq!(table.len(), 9);
    assert!(table.bucket_count() > 1);
}

#[test]
fn closest_nodes_are_sorted_by_xor_distance() {
    let local = NodeId::new([0; 20]);
    let target = node_id(3);
    let mut table = RoutingTable::new(local);
    table.insert(node_id(9), addr(9)).unwrap();
    table.insert(node_id(2), addr(2)).unwrap();
    table.insert(node_id(1), addr(1)).unwrap();

    let closest = table.closest_nodes(target, 2);

    assert_eq!(closest[0].id, node_id(2));
    assert_eq!(closest[1].id, node_id(1));
}

#[test]
fn reinserting_node_updates_address_and_marks_good() {
    let local = NodeId::new([0; 20]);
    let mut table = RoutingTable::new(local);
    table.insert(node_id(1), addr(1)).unwrap();
    table.mark_questionable(node_id(1)).unwrap();
    table.insert(node_id(1), addr(2)).unwrap();

    let node = table.closest_nodes(node_id(1), 1).remove(0);

    assert_eq!(node.addr, addr(2));
    assert_eq!(node.status, NodeStatus::Good);
}

fn node_id(first_byte: u8) -> NodeId {
    let mut bytes = [0_u8; 20];
    bytes[0] = first_byte;
    NodeId::new(bytes)
}

fn addr(last_octet: u8) -> NodeAddr {
    NodeAddr::new(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, last_octet)),
        6881,
    ))
}
