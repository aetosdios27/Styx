use std::{collections::BTreeMap, net::SocketAddr};

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_disk::{BlockLength, BlockOffset, BlockSpec, PieceIndex};
use styx_proto::{encode, BencodeValue, InfoHashV1};
use styx_runtime::{
    RuntimeCommand, RuntimeConfig, RuntimeEngine, RuntimeError, RuntimeEvent, TorrentCommand,
    TorrentId, TorrentStatus,
};

#[test]
fn runtime_engine_rejects_duplicate_torrent_add() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let duplicate = plan.clone();
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();

    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();
    let err = engine
        .apply(RuntimeCommand::AddPlan(Box::new(duplicate)))
        .unwrap_err();

    assert_eq!(err, RuntimeError::InvalidConfig("torrent already exists"));
}

#[test]
fn torrent_task_accepts_legal_pause_resume_cancel_transitions() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let id = plan.id;
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();

    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))
        .unwrap();
    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Pause))
        .unwrap();
    assert_eq!(engine.snapshot().torrents[0].status, TorrentStatus::Paused);
    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Resume))
        .unwrap();
    assert_eq!(
        engine.snapshot().torrents[0].status,
        TorrentStatus::Downloading
    );
    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Cancel))
        .unwrap();

    assert_eq!(
        engine.snapshot().torrents[0].status,
        TorrentStatus::Cancelled
    );
}

#[test]
fn torrent_task_rejects_resume_after_cancel() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let id = plan.id;
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();
    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Cancel))
        .unwrap();

    let err = engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Resume))
        .unwrap_err();

    assert_eq!(
        err,
        RuntimeError::InvalidConfig("illegal torrent state transition")
    );
}

#[test]
fn runtime_engine_preserves_terminal_events_when_event_queue_is_full() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let id = plan.id;
    let config = RuntimeConfig {
        limits: styx_runtime::RuntimeLimits {
            max_event_queue: 1,
            ..styx_runtime::RuntimeLimits::default()
        },
        ..RuntimeConfig::default()
    };
    let mut engine = RuntimeEngine::new(config).unwrap();

    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();
    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Cancel))
        .unwrap();

    assert!(matches!(
        engine.drain_events().last(),
        Some(RuntimeEvent::TaskCancelled { torrent }) if *torrent == id
    ));
}

#[test]
fn runtime_engine_ingests_dht_peers_as_download_sources() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let id = plan.id;
    let peer: SocketAddr = "127.0.0.1:51413".parse().unwrap();
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();

    let added = engine.add_dht_peers(id, vec![peer, peer]).unwrap();

    assert_eq!(added, 1);
    assert!(engine.drain_events().iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::DhtPeersDiscovered {
                torrent,
                peers
            } if *torrent == id && *peers == 1
        )
    }));
}

#[tokio::test]
async fn runtime_engine_verifies_later_piece_through_orchestration_path() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, two_piece_torrent_bytes()).unwrap();
    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let id = plan.id;
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();

    engine
        .accept_piece_blocks(
            id,
            PieceIndex::new(1),
            vec![(block(1, 0, 4, 4), Bytes::from_static(b"efgh"))],
        )
        .await
        .unwrap();

    let snapshot = engine.snapshot();
    assert_eq!(snapshot.torrents[0].verified_bytes, 4);
    assert!(engine
        .drain_events()
        .iter()
        .any(|event| matches!(event, RuntimeEvent::PieceVerified { piece: 1, .. })));
}

#[test]
fn apply_add_plan_routes_through_intent_pipeline() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let id = plan.id;
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();

    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();
    assert!(engine.has_torrent(id));
}

#[test]
fn apply_add_duplicate_plan_fails_validation() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let duplicate = plan.clone();
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();

    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();
    let result = engine.apply(RuntimeCommand::AddPlan(Box::new(duplicate)));
    assert!(result.is_err());
}

#[test]
fn apply_remove_routes_through_intent_pipeline() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let id = plan.id;
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();

    engine.apply(RuntimeCommand::Remove(id)).unwrap();
    assert!(!engine.has_torrent(id));
}

#[test]
fn apply_remove_unknown_returns_error() {
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let id = TorrentId::new(InfoHashV1::new([0u8; 20]));
    let result = engine.apply(RuntimeCommand::Remove(id));
    assert!(result.is_err());
}

#[test]
fn v2_only_torrent_returns_v2_not_supported() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("v2-only.torrent");
    std::fs::write(&torrent, v2_only_torrent_bytes()).unwrap();

    let err =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap_err();

    assert_eq!(err, RuntimeError::V2NotSupported);
}

#[test]
fn hybrid_torrent_with_v2_info_hash_parses_and_adds_to_engine() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("hybrid.torrent");
    std::fs::write(&torrent, hybrid_torrent_bytes()).unwrap();

    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();

    assert!(plan.info_hash_v2.is_some());
    assert_eq!(plan.info_hash_v2.unwrap().as_bytes().len(), 32);

    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();

    assert_eq!(engine.snapshot().torrents.len(), 1);
}

fn v2_only_torrent_bytes() -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"url-list".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"https://mirror.test/")),
    );
    let mut info = BTreeMap::new();
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"file.bin")),
    );
    info.insert(b"piece length".to_vec(), BencodeValue::Integer(16384));
    info.insert(b"length".to_vec(), BencodeValue::Integer(16384));
    info.insert(b"meta version".to_vec(), BencodeValue::Integer(2));
    let mut file_tree = BTreeMap::new();
    let mut file_entry = BTreeMap::new();
    file_entry.insert(b"length".to_vec(), BencodeValue::Integer(16384));
    file_entry.insert(
        b"pieces root".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(&[0u8; 32])),
    );
    let mut file_tree_inner = BTreeMap::new();
    file_tree_inner.insert(b"".to_vec(), BencodeValue::Dict(file_entry));
    file_tree.insert(b"file.bin".to_vec(), BencodeValue::Dict(file_tree_inner));
    info.insert(b"file tree".to_vec(), BencodeValue::Dict(file_tree));
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}

fn hybrid_torrent_bytes() -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"url-list".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"https://mirror.test/")),
    );
    let mut info = BTreeMap::new();
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"file.bin")),
    );
    info.insert(b"piece length".to_vec(), BencodeValue::Integer(4));
    info.insert(b"length".to_vec(), BencodeValue::Integer(4));
    info.insert(
        b"pieces".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(&Sha1::digest(b"abcd"))),
    );
    info.insert(b"meta version".to_vec(), BencodeValue::Integer(2));
    let mut file_tree = BTreeMap::new();
    let mut file_entry = BTreeMap::new();
    file_entry.insert(b"length".to_vec(), BencodeValue::Integer(4));
    file_entry.insert(
        b"pieces root".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(&[0u8; 32])),
    );
    let mut file_tree_inner = BTreeMap::new();
    file_tree_inner.insert(b"".to_vec(), BencodeValue::Dict(file_entry));
    file_tree.insert(b"file.bin".to_vec(), BencodeValue::Dict(file_tree_inner));
    info.insert(b"file tree".to_vec(), BencodeValue::Dict(file_tree));
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}

fn torrent_bytes() -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"url-list".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"https://mirror.test/")),
    );
    let mut info = BTreeMap::new();
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"file.bin")),
    );
    info.insert(b"piece length".to_vec(), BencodeValue::Integer(4));
    info.insert(b"length".to_vec(), BencodeValue::Integer(4));
    info.insert(
        b"pieces".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(&Sha1::digest(b"abcd"))),
    );
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}

fn two_piece_torrent_bytes() -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"url-list".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"https://mirror.test/")),
    );
    let mut info = BTreeMap::new();
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"file.bin")),
    );
    info.insert(b"piece length".to_vec(), BencodeValue::Integer(4));
    info.insert(b"length".to_vec(), BencodeValue::Integer(8));
    let mut pieces = Vec::new();
    pieces.extend_from_slice(&Sha1::digest(b"abcd"));
    pieces.extend_from_slice(&Sha1::digest(b"efgh"));
    info.insert(b"pieces".to_vec(), BencodeValue::Bytes(Bytes::from(pieces)));
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}

fn block(piece: u32, offset: u32, length: u32, piece_length: u32) -> BlockSpec {
    BlockSpec::new(
        PieceIndex::new(piece),
        BlockOffset::new(offset),
        BlockLength::new(length).unwrap(),
        piece_length,
    )
    .unwrap()
}
