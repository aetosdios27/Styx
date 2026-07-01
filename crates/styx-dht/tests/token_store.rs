use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bytes::Bytes;
use styx_dht::{CompactPeer, DhtError, InfoHash, PeerStore, TokenManager};

#[test]
fn token_validates_for_same_ip_and_current_secret() {
    let manager =
        TokenManager::with_secrets(Bytes::from_static(b"current"), Bytes::from_static(b"old"));
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
    let token = manager.issue(ip);

    assert!(manager.validate(ip, &token));
}

#[test]
fn token_rejects_different_source_ip() {
    let manager =
        TokenManager::with_secrets(Bytes::from_static(b"current"), Bytes::from_static(b"old"));
    let token = manager.issue(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));

    assert!(!manager.validate(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)), &token));
}

#[test]
fn token_accepts_previous_secret_during_rotation_window() {
    let mut manager =
        TokenManager::with_secrets(Bytes::from_static(b"current"), Bytes::from_static(b"old"));
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
    let old_token = manager.issue(ip);
    manager.rotate(Bytes::from_static(b"next"));

    assert!(manager.validate(ip, &old_token));
}

#[test]
fn peer_store_returns_announced_peers_for_info_hash() {
    let mut store = PeerStore::with_capacity(8);
    let hash = InfoHash::new([1; 20]);
    let peer = CompactPeer::new(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        6881,
    ));

    store.announce(hash, peer).unwrap();

    assert_eq!(store.peers(hash), vec![peer]);
}

#[test]
fn peer_store_rejects_info_hash_bucket_over_capacity() {
    let mut store = PeerStore::with_capacity(1);
    let hash = InfoHash::new([1; 20]);
    store
        .announce(
            hash,
            CompactPeer::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 1)),
        )
        .unwrap();
    let err = store
        .announce(
            hash,
            CompactPeer::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)), 2)),
        )
        .unwrap_err();

    assert_eq!(err, DhtError::PeerStoreFull);
}
