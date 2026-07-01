use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use styx_dht::{NodeAddr, NodeId, NodeStatus, RoutingTable};

#[test]
fn routing_table_marks_node_bad_after_repeated_failures() {
    let mut table = RoutingTable::new(NodeId::new([0; 20]));
    let id = NodeId::new([1; 20]);
    table.insert(id, addr(1)).unwrap();

    table.mark_failure(id).unwrap();
    table.mark_failure(id).unwrap();
    table.mark_failure(id).unwrap();

    assert_eq!(table.node(id).unwrap().status, NodeStatus::Bad);
}

#[test]
fn routing_table_success_resets_failure_count_and_marks_node_good() {
    let mut table = RoutingTable::new(NodeId::new([0; 20]));
    let id = NodeId::new([2; 20]);
    let now = Instant::now();
    table.insert(id, addr(2)).unwrap();
    table.mark_failure(id).unwrap();

    table.mark_seen(id, now).unwrap();

    let record = table.node(id).unwrap();
    assert_eq!(record.status, NodeStatus::Good);
    assert_eq!(record.failures, 0);
}

#[test]
fn routing_table_reports_refresh_target_for_stale_bucket() {
    let mut table = RoutingTable::new(NodeId::new([0; 20]));
    table.insert(NodeId::new([3; 20]), addr(3)).unwrap();
    let now = Instant::now();

    let target = table
        .stale_refresh_target(
            now + Duration::from_secs(16 * 60),
            Duration::from_secs(15 * 60),
        )
        .unwrap();

    assert_ne!(target, NodeId::new([0; 20]));
}

fn addr(last_octet: u8) -> NodeAddr {
    NodeAddr::new(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last_octet)),
        6881,
    ))
}
