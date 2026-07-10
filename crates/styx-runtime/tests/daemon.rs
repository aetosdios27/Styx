use std::{collections::BTreeMap, time::Duration};

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_app::{commands::CommandResponse, ControlCommand, TorrentStatus};
use styx_proto::{
    decode_handshake, encode, write_handshake, write_message, BencodeValue, ExtensionBits,
    Handshake, PeerId, PeerMessage, PEER_HANDSHAKE_LEN,
};
use styx_runtime::{
    DaemonConfig, DaemonRuntime, PersistentState, PersistentStore, PersistentTorrent,
    PersistentTorrentSource, PersistentTorrentState, RuntimeConfig,
    PERSISTENT_STATE_SCHEMA_VERSION,
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
async fn empty_daemon_shutdown_completes_with_best_effort_lsd_configured() {
    let root = tempfile::tempdir().unwrap();
    let mut config = daemon_config(root.path());
    config.runtime_config.dht.enabled = false;
    let daemon = DaemonRuntime::start(config).await.unwrap();

    tokio::time::timeout(Duration::from_secs(2), daemon.shutdown())
        .await
        .expect("daemon shutdown exceeded its external deadline")
        .unwrap();
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
            schema_version: PERSISTENT_STATE_SCHEMA_VERSION,
            torrents: vec![PersistentTorrent {
                source: PersistentTorrentSource::File { path: torrent },
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

#[tokio::test]
async fn daemon_restart_can_serve_block_from_restored_completed_torrent() {
    let temp = tempfile::tempdir().unwrap();
    let piece_bytes = Bytes::from_static(b"abcd");
    let mut leecher = serve_interested_leecher(piece_bytes.clone()).await;
    let tracker = serve_tracker(vec![leecher.addr]).await;
    let torrent = temp.path().join("seedable.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(
        &torrent,
        torrent_with_announce(&[piece_bytes.as_ref()], tracker.as_str()),
    )
    .unwrap();
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("file.bin"), piece_bytes.as_ref()).unwrap();
    let store = PersistentStore::open(temp.path().join("state")).unwrap();
    store
        .save(&PersistentState {
            schema_version: PERSISTENT_STATE_SCHEMA_VERSION,
            torrents: vec![PersistentTorrent {
                source: PersistentTorrentSource::File { path: torrent },
                destination,
                state: PersistentTorrentState::Complete,
                added_at_unix: 1,
                completed_at_unix: Some(2),
            }],
        })
        .unwrap();

    let daemon = DaemonRuntime::start(daemon_config(temp.path()))
        .await
        .unwrap();
    for _ in 0..100 {
        if let Ok(received) = leecher.rx.try_recv() {
            daemon.shutdown().await.unwrap();
            assert_eq!(received, piece_bytes);
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    daemon.shutdown().await.unwrap();

    panic!("restored completed torrent did not serve the interested leecher");
}

fn daemon_config(root: &std::path::Path) -> DaemonConfig {
    DaemonConfig {
        state_dir: root.join("state"),
        socket_path: root.join("styx.sock"),
        tick_interval: Duration::from_millis(10),
        runtime_config: RuntimeConfig::default(),
    }
}

fn torrent_with_announce(chunks: &[&[u8]], announce: &str) -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"announce".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(announce.as_bytes())),
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

async fn serve_tracker(peers: Vec<std::net::SocketAddr>) -> url::Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await.unwrap();
        let body = announce_response(&peers);
        stream
            .write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(&body).await.unwrap();
    });
    url::Url::parse(&format!("http://{addr}/announce")).unwrap()
}

fn announce_response(peers: &[std::net::SocketAddr]) -> Vec<u8> {
    let mut dict = BTreeMap::new();
    dict.insert(b"complete".to_vec(), BencodeValue::Integer(1));
    dict.insert(b"incomplete".to_vec(), BencodeValue::Integer(0));
    dict.insert(b"interval".to_vec(), BencodeValue::Integer(1800));
    dict.insert(
        b"peers".to_vec(),
        BencodeValue::Bytes(Bytes::from(compact_peers(peers))),
    );
    encode(&BencodeValue::Dict(dict))
}

fn compact_peers(peers: &[std::net::SocketAddr]) -> Vec<u8> {
    let mut out = Vec::new();
    for peer in peers {
        if let std::net::SocketAddr::V4(v4) = peer {
            out.extend_from_slice(&v4.ip().octets());
            out.extend_from_slice(&v4.port().to_be_bytes());
        }
    }
    out
}

struct LeecherProbe {
    addr: std::net::SocketAddr,
    rx: tokio::sync::oneshot::Receiver<Bytes>,
}

async fn serve_interested_leecher(expected_piece: Bytes) -> LeecherProbe {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut handshake_bytes = [0u8; PEER_HANDSHAKE_LEN];
        stream.read_exact(&mut handshake_bytes).await.unwrap();
        let incoming = decode_handshake(&handshake_bytes).unwrap();
        write_handshake(
            &mut stream,
            &Handshake {
                reserved: ExtensionBits::default(),
                info_hash: incoming.info_hash,
                peer_id: PeerId::new([8u8; 20]),
            },
        )
        .await
        .unwrap();
        write_message(&mut stream, &PeerMessage::Interested)
            .await
            .unwrap();
        let first = styx_proto::read_message(&mut stream, styx_proto::DEFAULT_MAX_PEER_FRAME_LEN)
            .await
            .unwrap();
        assert_eq!(first, PeerMessage::Unchoke);
        write_message(
            &mut stream,
            &PeerMessage::Request {
                index: 0,
                begin: 0,
                length: expected_piece.len() as u32,
            },
        )
        .await
        .unwrap();
        let piece = styx_proto::read_message(&mut stream, styx_proto::DEFAULT_MAX_PEER_FRAME_LEN)
            .await
            .unwrap();
        let PeerMessage::Piece { block, .. } = piece else {
            panic!("expected piece message, got {piece:?}");
        };
        let _ = tx.send(block);
    });
    LeecherProbe { addr, rx }
}
