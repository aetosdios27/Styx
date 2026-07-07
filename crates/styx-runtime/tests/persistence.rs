use std::path::PathBuf;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use styx_app::TorrentRuntime;
use styx_proto::{encode, BencodeValue};
use styx_runtime::{
    AppRuntime, PersistentState, PersistentStore, PersistentTorrent, PersistentTorrentState,
    RuntimeConfig, RuntimeError,
};

#[test]
fn persistent_state_round_trips_torrent_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let store = PersistentStore::open(temp.path()).unwrap();
    let source_path = PathBuf::from("/tmp/styx/sample.torrent");
    let destination = PathBuf::from("/tmp/styx/downloads");
    let state = PersistentState {
        schema_version: 1,
        torrents: vec![PersistentTorrent {
            source_path: source_path.clone(),
            destination: destination.clone(),
            state: PersistentTorrentState::Downloading,
            added_at_unix: 1_725_000_000,
            completed_at_unix: None,
        }],
    };

    store.save(&state).unwrap();

    let restored = store.load().unwrap();
    assert_eq!(restored, state);
}

#[test]
fn persistent_store_rejects_unknown_schema_version() {
    let temp = tempfile::tempdir().unwrap();
    let store = PersistentStore::open(temp.path()).unwrap();
    std::fs::write(
        temp.path().join("state.json"),
        r#"{"schema_version":999,"torrents":[]}"#,
    )
    .unwrap();

    let err = store.load().unwrap_err();

    assert_eq!(
        err,
        RuntimeError::Persistence("unsupported persistent state schema version")
    );
}

#[test]
fn persistent_store_rejects_corrupt_json_without_deleting_file() {
    let temp = tempfile::tempdir().unwrap();
    let store = PersistentStore::open(temp.path()).unwrap();
    let path = temp.path().join("state.json");
    std::fs::write(&path, b"{not-json").unwrap();

    let err = store.load().unwrap_err();

    assert_eq!(
        err,
        RuntimeError::Persistence("invalid persistent state json")
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"{not-json");
}

#[tokio::test]
async fn restore_readds_paused_torrent_without_starting_background_download() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, torrent_from_chunks(&[b"abcd".as_slice()])).unwrap();
    let state = PersistentState {
        schema_version: 1,
        torrents: vec![PersistentTorrent {
            source_path: torrent,
            destination,
            state: PersistentTorrentState::Paused,
            added_at_unix: 1,
            completed_at_unix: None,
        }],
    };

    let mut runtime = AppRuntime::restore_from_state(RuntimeConfig::default(), state)
        .await
        .unwrap();

    assert_eq!(
        runtime.snapshot().torrents[0].status,
        styx_app::TorrentStatus::Paused
    );
}

#[tokio::test]
async fn restore_completed_torrent_reverifies_existing_data_before_seeding() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, torrent_from_chunks(&[b"abcd".as_slice()])).unwrap();
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("file.bin"), b"abcd").unwrap();
    let state = PersistentState {
        schema_version: 1,
        torrents: vec![PersistentTorrent {
            source_path: torrent,
            destination,
            state: PersistentTorrentState::Complete,
            added_at_unix: 1,
            completed_at_unix: Some(2),
        }],
    };

    let mut runtime = AppRuntime::restore_from_state(RuntimeConfig::default(), state)
        .await
        .unwrap();
    let torrent = &runtime.snapshot().torrents[0];

    assert_eq!(torrent.status, styx_app::TorrentStatus::Seeding);
    assert_eq!(torrent.progress, 1.0);
}

#[tokio::test]
async fn restore_missing_torrent_file_returns_typed_error() {
    let temp = tempfile::tempdir().unwrap();
    let state = PersistentState {
        schema_version: 1,
        torrents: vec![PersistentTorrent {
            source_path: temp.path().join("missing.torrent"),
            destination: temp.path().join("downloads"),
            state: PersistentTorrentState::Downloading,
            added_at_unix: 1,
            completed_at_unix: None,
        }],
    };

    let err = AppRuntime::restore_from_state(RuntimeConfig::default(), state)
        .await
        .unwrap_err();

    assert_eq!(
        err,
        RuntimeError::Persistence("persistent torrent source is missing")
    );
}

#[tokio::test]
async fn persistent_state_returns_restored_torrent_intent() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, torrent_from_chunks(&[b"abcd".as_slice()])).unwrap();
    let state = PersistentState {
        schema_version: 1,
        torrents: vec![PersistentTorrent {
            source_path: torrent,
            destination,
            state: PersistentTorrentState::Downloading,
            added_at_unix: 11,
            completed_at_unix: None,
        }],
    };
    let mut runtime = AppRuntime::restore_from_state(RuntimeConfig::default(), state.clone())
        .await
        .unwrap();

    assert_eq!(runtime.persistent_state(), state);
}

fn torrent_from_chunks(chunks: &[&[u8]]) -> Vec<u8> {
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
    info.insert(
        b"length".to_vec(),
        BencodeValue::Integer(chunks.iter().map(|chunk| chunk.len() as i64).sum()),
    );
    let mut pieces = Vec::new();
    for chunk in chunks {
        pieces.extend_from_slice(&Sha1::digest(chunk));
    }
    info.insert(b"pieces".to_vec(), BencodeValue::Bytes(Bytes::from(pieces)));
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}
