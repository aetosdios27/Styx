use std::path::PathBuf;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use styx_app::TorrentRuntime;
use styx_proto::{encode, BencodeValue};
use styx_runtime::{
    AppRuntime, PersistentAppRuntime, PersistentState, PersistentStore, PersistentTorrent,
    PersistentTorrentState, RuntimeConfig, RuntimeError,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

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
async fn restore_completed_torrent_with_missing_data_does_not_seed() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, torrent_from_chunks(&[b"abcd".as_slice()])).unwrap();
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

    assert_eq!(
        runtime.snapshot().torrents[0].status,
        styx_app::TorrentStatus::Paused
    );
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

#[tokio::test]
async fn add_command_persists_new_torrent_record() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, torrent_from_chunks(&[b"abcd".as_slice()])).unwrap();
    let store = PersistentStore::open(temp.path().join("state")).unwrap();
    let mut runtime = PersistentAppRuntime::open(RuntimeConfig::default(), store.clone())
        .await
        .unwrap();

    runtime
        .apply_and_persist(styx_app::ControlCommand::Add {
            source: torrent.clone(),
            destination: Some(destination.clone()),
        })
        .unwrap();

    let state = store.load().unwrap();
    assert_eq!(state.torrents[0].source_path, torrent);
    assert_eq!(state.torrents[0].destination, destination);
    assert_eq!(state.torrents[0].state, PersistentTorrentState::Downloading);
}

#[tokio::test]
async fn pause_and_resume_update_persisted_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, torrent_from_chunks(&[b"abcd".as_slice()])).unwrap();
    let store = PersistentStore::open(temp.path().join("state")).unwrap();
    let mut runtime = PersistentAppRuntime::open(RuntimeConfig::default(), store.clone())
        .await
        .unwrap();
    let added = runtime
        .apply_and_persist(styx_app::ControlCommand::Add {
            source: torrent,
            destination: Some(destination),
        })
        .unwrap();
    let styx_app::commands::CommandResponse::TorrentAdded { info_hash, .. } = added else {
        panic!("expected add response");
    };

    runtime
        .apply_and_persist(styx_app::ControlCommand::Pause { info_hash })
        .unwrap();
    assert_eq!(
        store.load().unwrap().torrents[0].state,
        PersistentTorrentState::Paused
    );

    runtime
        .apply_and_persist(styx_app::ControlCommand::Resume { info_hash })
        .unwrap();
    assert_eq!(
        store.load().unwrap().torrents[0].state,
        PersistentTorrentState::Downloading
    );
}

#[tokio::test]
async fn status_command_does_not_create_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let store = PersistentStore::open(temp.path().join("state")).unwrap();
    let mut runtime = PersistentAppRuntime::open(RuntimeConfig::default(), store.clone())
        .await
        .unwrap();

    runtime
        .apply_and_persist(styx_app::ControlCommand::Status)
        .unwrap();

    assert!(!store.state_path().exists());
}

#[tokio::test]
async fn completion_event_persists_complete_state() {
    let temp = tempfile::tempdir().unwrap();
    let web_seed = serve_web_seed(Bytes::from_static(b"abcd")).await;
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(
        &torrent,
        torrent_with_web_seed(&[b"abcd".as_slice()], web_seed.as_str()),
    )
    .unwrap();
    let store = PersistentStore::open(temp.path().join("state")).unwrap();
    let mut runtime = PersistentAppRuntime::open(RuntimeConfig::default(), store.clone())
        .await
        .unwrap();
    runtime
        .apply_and_persist(styx_app::ControlCommand::Add {
            source: torrent,
            destination: Some(destination),
        })
        .unwrap();

    for _ in 0..100 {
        runtime.tick_and_persist().unwrap();
        if store.load().unwrap().torrents[0].state == PersistentTorrentState::Complete {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert_eq!(
        store.load().unwrap().torrents[0].state,
        PersistentTorrentState::Complete
    );
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

fn torrent_with_web_seed(chunks: &[&[u8]], web_seed: &str) -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"url-list".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(web_seed.as_bytes())),
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

async fn serve_web_seed(piece_bytes: Bytes) -> url::Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await.unwrap();
        let response = format!(
            "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes 0-{}/{}\r\n\r\n",
            piece_bytes.len(),
            piece_bytes.len() - 1,
            piece_bytes.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.write_all(&piece_bytes).await.unwrap();
    });
    url::Url::parse(&format!("http://{addr}/file.bin")).unwrap()
}
