use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use styx_dht::{NodeAddr, NodeId, RoutingCache, RoutingCacheEntry};

#[test]
fn routing_cache_round_trips_known_node_addresses_without_local_identity() {
    let local_id = NodeId::new([1; 20]);
    let now = Instant::now();
    let entries = vec![RoutingCacheEntry {
        id: NodeId::new([2; 20]),
        addr: addr(2),
        last_seen: now,
    }];

    let cache = RoutingCache::from_entries(local_id, entries.clone());

    assert_eq!(cache.local_id(), None);
    assert_eq!(cache.entries(now, Duration::from_secs(60)), entries);
}

#[test]
fn routing_cache_expires_stale_entries() {
    let now = Instant::now();
    let cache = RoutingCache::from_entries(
        NodeId::new([1; 20]),
        vec![RoutingCacheEntry {
            id: NodeId::new([3; 20]),
            addr: addr(3),
            last_seen: now,
        }],
    );

    assert!(cache
        .entries(now + Duration::from_secs(120), Duration::from_secs(60))
        .is_empty());
}

fn addr(last_octet: u8) -> NodeAddr {
    NodeAddr::new(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last_octet)),
        6881,
    ))
}
