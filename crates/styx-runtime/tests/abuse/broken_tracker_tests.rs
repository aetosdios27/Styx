use std::net::SocketAddr;
use std::time::Duration;

use styx_proto::{InfoHashV1, PeerId};
use styx_tracker::{AnnounceEvent, AnnounceRequest, HttpTrackerClient, TrackerError};
use url::Url;

use crate::abuse::mock_tracker::{MockTracker, MockTrackerBehavior};

fn announce_request(peer_addr: SocketAddr) -> AnnounceRequest {
    AnnounceRequest {
        info_hash: InfoHashV1::new([1; 20]),
        peer_id: PeerId::new([2; 20]),
        port: peer_addr.port(),
        uploaded: 0,
        downloaded: 0,
        left: 1000,
        event: Some(AnnounceEvent::Started),
        compact: true,
        numwant: Some(50),
        key: None,
    }
}

#[tokio::test]
async fn broken_tracker_normal_returns_peers() {
    let peer1: SocketAddr = "192.168.1.1:6881".parse().unwrap();
    let peer2: SocketAddr = "192.168.1.2:6882".parse().unwrap();

    let tracker = MockTracker::new(MockTrackerBehavior::Normal(vec![peer1, peer2]));
    let (addr, handle) = tracker.serve().await;

    let client = HttpTrackerClient::new(65536);
    let url = Url::parse(&format!("http://{addr}/announce")).unwrap();
    let request = announce_request(addr);

    let response = client.announce(&url, &request).await.unwrap();

    assert_eq!(response.peers.len(), 2);
    let mut addrs: Vec<SocketAddr> = response.peers.iter().map(|p| p.addr).collect();
    addrs.sort();
    assert_eq!(addrs, vec![peer1, peer2]);

    handle.await.unwrap();
}

#[tokio::test]
async fn broken_tracker_http500_returns_error() {
    let tracker = MockTracker::new(MockTrackerBehavior::Http500);
    let (addr, handle) = tracker.serve().await;

    let client = HttpTrackerClient::new(65536);
    let url = Url::parse(&format!("http://{addr}/announce")).unwrap();
    let request = announce_request(addr);

    let result = client.announce(&url, &request).await;

    assert!(
        result.is_err(),
        "expected an error for HTTP 500, got {:?}",
        result
    );

    handle.await.unwrap();
}

#[tokio::test]
async fn broken_tracker_malformed_bencode_returns_error() {
    let tracker = MockTracker::new(MockTrackerBehavior::MalformedBencode);
    let (addr, handle) = tracker.serve().await;

    let client = HttpTrackerClient::new(65536);
    let url = Url::parse(&format!("http://{addr}/announce")).unwrap();
    let request = announce_request(addr);

    let result = client.announce(&url, &request).await;

    assert!(matches!(result, Err(TrackerError::Bencode(_))));

    handle.await.unwrap();
}

#[tokio::test]
async fn broken_tracker_empty_peer_list_returns_empty_peers() {
    let tracker = MockTracker::new(MockTrackerBehavior::EmptyPeerList);
    let (addr, handle) = tracker.serve().await;

    let client = HttpTrackerClient::new(65536);
    let url = Url::parse(&format!("http://{addr}/announce")).unwrap();
    let request = announce_request(addr);

    let response = client.announce(&url, &request).await.unwrap();

    assert!(response.peers.is_empty());

    handle.await.unwrap();
}

#[tokio::test]
async fn broken_tracker_delayed_response_times_out() {
    let tracker = MockTracker::new(MockTrackerBehavior::Delayed(2000));
    let (addr, handle) = tracker.serve().await;

    let client = HttpTrackerClient::new(65536);
    let url = Url::parse(&format!("http://{addr}/announce")).unwrap();
    let request = announce_request(addr);

    let result =
        tokio::time::timeout(Duration::from_secs(1), client.announce(&url, &request)).await;

    assert!(result.is_err(), "expected timeout, got {:?}", result);

    handle.await.unwrap();
}

#[tokio::test]
async fn broken_tracker_never_responds_times_out() {
    let tracker = MockTracker::new(MockTrackerBehavior::NeverResponds);
    let (addr, handle) = tracker.serve().await;

    let client = HttpTrackerClient::new(65536);
    let url = Url::parse(&format!("http://{addr}/announce")).unwrap();
    let request = announce_request(addr);

    let result = client.announce(&url, &request).await;

    assert!(
        result.is_err(),
        "expected error for dropped connection, got {:?}",
        result
    );

    handle.await.unwrap();
}
