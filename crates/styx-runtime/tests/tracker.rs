use std::{collections::BTreeMap, net::SocketAddr};

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_proto::{encode, BencodeValue, PeerId};
use styx_runtime::{
    build_started_announce, load_torrent_plan, select_peer_candidates, SmokeConfig,
};
use styx_tracker::{AnnounceEvent, AnnounceResponse, TrackerPeer};

#[test]
fn build_started_announce_uses_smoke_session_counters() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, single_file_torrent()).unwrap();
    let plan = load_torrent_plan(
        &torrent,
        temp.path().join("downloads"),
        &SmokeConfig::default(),
    )
    .unwrap();
    let peer_id = PeerId::new([9; 20]);

    let request = build_started_announce(&plan, peer_id, 6881, 11);

    assert_eq!(request.info_hash, plan.info_hash);
    assert_eq!(request.peer_id, peer_id);
    assert_eq!(request.port, 6881);
    assert_eq!(request.uploaded, 0);
    assert_eq!(request.downloaded, 0);
    assert_eq!(request.left, 8);
    assert_eq!(request.event, Some(AnnounceEvent::Started));
    assert!(request.compact);
    assert_eq!(request.numwant, Some(11));
}

#[test]
fn select_peer_candidates_deduplicates_and_skips_unusable_endpoints() {
    let good: SocketAddr = "198.51.100.10:6881".parse().unwrap();
    let zero_port: SocketAddr = "198.51.100.11:0".parse().unwrap();
    let unspecified: SocketAddr = "0.0.0.0:6881".parse().unwrap();
    let response = AnnounceResponse {
        interval: 1800,
        min_interval: None,
        tracker_id: None,
        seeders: None,
        leechers: None,
        warning_message: None,
        peers: vec![
            TrackerPeer {
                addr: good,
                peer_id: None,
            },
            TrackerPeer {
                addr: zero_port,
                peer_id: None,
            },
            TrackerPeer {
                addr: unspecified,
                peer_id: None,
            },
            TrackerPeer {
                addr: good,
                peer_id: None,
            },
        ],
    };

    assert_eq!(select_peer_candidates(&response, 10), vec![good]);
}

fn single_file_torrent() -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"announce".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"http://tracker.test/announce")),
    );
    top.insert(b"info".to_vec(), info_dict());
    encode(&BencodeValue::Dict(top))
}

fn info_dict() -> BencodeValue {
    let mut info = BTreeMap::new();
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"file.bin")),
    );
    info.insert(b"piece length".to_vec(), BencodeValue::Integer(4));
    info.insert(b"length".to_vec(), BencodeValue::Integer(8));
    info.insert(b"pieces".to_vec(), BencodeValue::Bytes(piece_hashes()));
    BencodeValue::Dict(info)
}

fn piece_hashes() -> Bytes {
    let mut bytes = Vec::new();
    for chunk in [b"abcd".as_slice(), b"efgh".as_slice()] {
        bytes.extend_from_slice(&Sha1::digest(chunk));
    }
    Bytes::from(bytes)
}
