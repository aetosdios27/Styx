use std::collections::BTreeMap;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_disk::PieceIndex;
use styx_proto::{encode, BencodeValue};
use styx_runtime::{load_torrent_plan, RuntimeError, SmokeConfig, TorrentPlan};

#[test]
fn load_torrent_plan_selects_first_piece_and_total_left() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(
        &torrent,
        single_file_torrent(Some("http://tracker.test/announce")),
    )
    .unwrap();

    let plan = load_torrent_plan(
        &torrent,
        temp.path().join("downloads"),
        &SmokeConfig::default(),
    )
    .unwrap();

    assert_eq!(plan.target_piece.get(), 0);
    assert_eq!(plan.total_size, 8);
    assert_eq!(plan.left, 8);
    assert_eq!(
        plan.announce_urls[0].as_str(),
        "http://tracker.test/announce"
    );
    assert_eq!(plan.disk_plan.piece_count(), 2);
}

#[test]
fn load_torrent_plan_rejects_torrent_without_http_tracker() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, single_file_torrent(Some("udp://tracker.test:80"))).unwrap();

    let err = load_torrent_plan(
        &torrent,
        temp.path().join("downloads"),
        &SmokeConfig::default(),
    )
    .unwrap_err();

    assert_eq!(err, RuntimeError::NoHttpTracker);
}

#[test]
fn load_torrent_plan_uses_announce_list_http_url_when_primary_is_absent() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_with_announce_list()).unwrap();

    let plan = load_torrent_plan(
        &torrent,
        temp.path().join("downloads"),
        &SmokeConfig::default(),
    )
    .unwrap();

    assert_eq!(
        plan.announce_urls[0].as_str(),
        "http://tracker.test/announce"
    );
}

#[test]
fn load_torrent_plan_accepts_url_list_web_seed_without_tracker() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_with_url_list()).unwrap();

    let plan = load_torrent_plan(
        &torrent,
        temp.path().join("downloads"),
        &SmokeConfig::default(),
    )
    .unwrap();

    assert!(plan.announce_urls.is_empty());
    assert_eq!(plan.web_seed_urls[0].as_str(), "https://mirror.test/iso/");
}

#[test]
fn torrent_plan_exposes_piece_count_piece_lengths_and_sources() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_with_url_list()).unwrap();

    let plan = TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();

    assert_eq!(plan.name, "file.bin");
    assert_eq!(plan.total_size, 8);
    assert_eq!(plan.piece_count(), 2);
    assert_eq!(plan.piece_length(PieceIndex::new(0)).unwrap(), 4);
    assert_eq!(plan.piece_length(PieceIndex::new(1)).unwrap(), 4);
    assert!(plan.announce_urls.is_empty());
    assert_eq!(plan.web_seed_urls[0].as_str(), "https://mirror.test/iso/");
}

#[test]
fn torrent_plan_rejects_torrents_without_any_http_source() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, single_file_torrent(Some("udp://tracker.test:80"))).unwrap();

    let err = TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap_err();

    assert_eq!(err, RuntimeError::NoHttpTracker);
}

fn single_file_torrent(announce: Option<&str>) -> Vec<u8> {
    let mut top = BTreeMap::new();
    if let Some(announce) = announce {
        top.insert(
            b"announce".to_vec(),
            BencodeValue::Bytes(Bytes::copy_from_slice(announce.as_bytes())),
        );
    }
    top.insert(b"info".to_vec(), info_dict());
    encode(&BencodeValue::Dict(top))
}

fn torrent_with_announce_list() -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"announce-list".to_vec(),
        BencodeValue::List(vec![BencodeValue::List(vec![BencodeValue::Bytes(
            Bytes::from_static(b"http://tracker.test/announce"),
        )])]),
    );
    top.insert(b"info".to_vec(), info_dict());
    encode(&BencodeValue::Dict(top))
}

fn torrent_with_url_list() -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"url-list".to_vec(),
        BencodeValue::List(vec![BencodeValue::Bytes(Bytes::from_static(
            b"https://mirror.test/iso/",
        ))]),
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
