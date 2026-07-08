use std::time::{Duration, Instant};

use styx_proto::InfoHashV1;
use styx_runtime::{
    RateCounter, RuntimeEvent, RuntimeSnapshot, TorrentId, TorrentSnapshot, TorrentStatus,
};

#[test]
fn torrent_snapshot_progress_uses_verified_bytes_only() {
    let torrent = TorrentSnapshot::new(TorrentId::new(InfoHashV1::new([3; 20])), "sample.iso", 100)
        .with_verified_bytes(25)
        .with_downloaded_bytes(90);

    assert_eq!(torrent.progress(), 0.25);
}

#[test]
fn torrent_snapshot_share_ratio_uses_uploaded_over_total_when_complete() {
    let mut torrent =
        TorrentSnapshot::new(TorrentId::new(InfoHashV1::new([6; 20])), "sample.iso", 100)
            .with_verified_bytes(100)
            .with_uploaded_bytes(250);
    torrent.status = TorrentStatus::Seeding;

    assert_eq!(torrent.uploaded_bytes, 250);
    assert_eq!(torrent.share_ratio(), 2.5);
}

#[test]
fn rate_counter_reports_bytes_inside_window() {
    let start = Instant::now();
    let mut counter = RateCounter::new(Duration::from_secs(5)).unwrap();
    counter.record(start, 100);
    counter.record(start + Duration::from_secs(3), 400);
    counter.record(start + Duration::from_secs(7), 800);

    assert_eq!(
        counter.bytes_per_second(start + Duration::from_secs(7)),
        240
    );
}

#[test]
fn runtime_snapshot_totals_count_torrents_and_peers() {
    let snapshot = RuntimeSnapshot {
        torrents: vec![TorrentSnapshot::new(
            TorrentId::new(InfoHashV1::new([4; 20])),
            "sample.iso",
            100,
        )],
        peers: vec![styx_runtime::PeerSnapshot {
            torrent: TorrentId::new(InfoHashV1::new([4; 20])),
            source: "peer:127.0.0.1:6881".to_owned(),
            progress: 0.5,
            down_rate: 128,
            up_rate: 0,
        }],
        events: Vec::new(),
    };

    assert_eq!(snapshot.torrent_count(), 1);
    assert_eq!(snapshot.peer_count(), 1);
}

#[test]
fn runtime_event_variants_are_stable() {
    let id = TorrentId::new(InfoHashV1::new([5; 20]));
    let event = RuntimeEvent::StateChanged {
        torrent: id,
        from: TorrentStatus::Checking,
        to: TorrentStatus::Downloading,
    };

    assert_eq!(event.kind(), "state_changed");
}
