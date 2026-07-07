use std::{collections::BTreeMap, time::Duration};

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_app::{commands::CommandResponse, ControlCommand, TorrentStatus};
use styx_proto::{encode, BencodeValue};
use styx_runtime::{DaemonConfig, DaemonRuntime, PersistentStore, RuntimeConfig};

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
