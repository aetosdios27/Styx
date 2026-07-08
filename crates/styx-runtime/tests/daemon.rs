use std::{collections::BTreeMap, time::Duration};

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_app::{commands::CommandResponse, ControlCommand, TorrentStatus};
use styx_proto::{encode, BencodeValue};
use styx_runtime::{
    DaemonConfig, DaemonRuntime, PersistentState, PersistentStore, PersistentTorrent,
    PersistentTorrentState, RuntimeConfig,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn daemon_start_loads_empty_state_and_reports_status() {
    let temp = tempfile::tempdir().unwrap();
    let config = daemon_config(temp.path());

    let daemon = DaemonRuntime::start(config).await.unwrap();
    let status = daemon.status().await.unwrap();
    daemon.shutdown().await.unwrap();

    assert_eq!(status.torrent_count, 0);
}

#[tokio::test]
async fn daemon_apply_add_persists_and_status_returns_torrent() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, torrent_from_chunks(&[b"abcd".as_slice()])).unwrap();
    let config = daemon_config(temp.path());
    let store = PersistentStore::open(temp.path().join("state")).unwrap();

    let daemon = DaemonRuntime::start(config).await.unwrap();
    daemon
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(destination),
        })
        .await
        .unwrap();
    let response = daemon.apply(ControlCommand::Status).await.unwrap();
    daemon.shutdown().await.unwrap();

    let CommandResponse::Status { snapshot } = response else {
        panic!("expected status response");
    };
    assert_eq!(snapshot.torrents.len(), 1);
    assert_eq!(store.load().unwrap().torrents.len(), 1);
}

#[tokio::test]
async fn daemon_shutdown_persists_latest_state() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, torrent_from_chunks(&[b"abcd".as_slice()])).unwrap();
    let config = daemon_config(temp.path());
    let store = PersistentStore::open(temp.path().join("state")).unwrap();

    let daemon = DaemonRuntime::start(config).await.unwrap();
    daemon
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(destination),
        })
        .await
        .unwrap();
    daemon.shutdown().await.unwrap();

    assert_eq!(store.load().unwrap().torrents.len(), 1);
}

#[tokio::test]
async fn daemon_restart_restores_torrent_from_state() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, torrent_from_chunks(&[b"abcd".as_slice()])).unwrap();

    let daemon = DaemonRuntime::start(daemon_config(temp.path()))
        .await
        .unwrap();
    daemon
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(destination),
        })
        .await
        .unwrap();
    daemon.shutdown().await.unwrap();

    let daemon = DaemonRuntime::start(daemon_config(temp.path()))
        .await
        .unwrap();
    let response = daemon.apply(ControlCommand::Status).await.unwrap();
    daemon.shutdown().await.unwrap();

    let CommandResponse::Status { snapshot } = response else {
        panic!("expected status response");
    };
    assert_eq!(snapshot.torrents[0].status, TorrentStatus::Checking);
}

#[tokio::test]
async fn restart_after_unclean_abort_restores_last_persisted_add() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, torrent_from_chunks(&[b"abcd".as_slice()])).unwrap();

    let daemon = DaemonRuntime::start(daemon_config(temp.path()))
        .await
        .unwrap();
    daemon
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(destination),
        })
        .await
        .unwrap();
    daemon.abort().await;

    let daemon = DaemonRuntime::start(daemon_config(temp.path()))
        .await
        .unwrap();
    let response = daemon.apply(ControlCommand::Status).await.unwrap();
    daemon.shutdown().await.unwrap();

    let CommandResponse::Status { snapshot } = response else {
        panic!("expected status response");
    };
    assert_eq!(snapshot.torrents.len(), 1);
}

#[tokio::test]
async fn restart_remembers_completed_torrent_after_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let web_seed = serve_web_seed(Bytes::from_static(b"abcd")).await;
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(
        &torrent,
        torrent_with_web_seed(&[b"abcd".as_slice()], web_seed.as_str()),
    )
    .unwrap();

    let daemon = DaemonRuntime::start(daemon_config(temp.path()))
        .await
        .unwrap();
    daemon
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(destination),
        })
        .await
        .unwrap();
    for _ in 0..100 {
        let response = daemon.apply(ControlCommand::Status).await.unwrap();
        let CommandResponse::Status { snapshot } = response else {
            panic!("expected status response");
        };
        if snapshot.torrents[0].status == TorrentStatus::Seeding {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    daemon.shutdown().await.unwrap();

    let daemon = DaemonRuntime::start(daemon_config(temp.path()))
        .await
        .unwrap();
    let response = daemon.apply(ControlCommand::Status).await.unwrap();
    daemon.shutdown().await.unwrap();

    let CommandResponse::Status { snapshot } = response else {
        panic!("expected status response");
    };
    assert_eq!(snapshot.torrents[0].status, TorrentStatus::Seeding);
}

#[tokio::test]
async fn restart_rechecks_partial_piece_state_before_continuing() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(
        &torrent,
        torrent_from_chunks(&[b"abcd".as_slice(), b"efgh".as_slice()]),
    )
    .unwrap();
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("file.bin"), b"abcdXXXX").unwrap();
    let store = PersistentStore::open(temp.path().join("state")).unwrap();
    store
        .save(&PersistentState {
            schema_version: 1,
            torrents: vec![PersistentTorrent {
                source_path: torrent,
                destination,
                state: PersistentTorrentState::Downloading,
                added_at_unix: 1,
                completed_at_unix: None,
            }],
        })
        .unwrap();

    let daemon = DaemonRuntime::start(daemon_config(temp.path()))
        .await
        .unwrap();
    let response = daemon.apply(ControlCommand::Status).await.unwrap();
    daemon.shutdown().await.unwrap();

    let CommandResponse::Status { snapshot } = response else {
        panic!("expected status response");
    };
    assert_eq!(snapshot.torrents[0].progress, 0.5);
}

fn daemon_config(root: &std::path::Path) -> DaemonConfig {
    DaemonConfig {
        state_dir: root.join("state"),
        socket_path: root.join("styx.sock"),
        tick_interval: Duration::from_millis(10),
        runtime_config: RuntimeConfig::default(),
    }
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
