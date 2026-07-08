use std::{collections::BTreeMap, net::SocketAddr};

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_app::TorrentRuntime;
use styx_proto::{
    decode_handshake, encode, write_handshake, write_message, BencodeValue, ExtensionBits,
    Handshake, PeerId, PeerMessage, PEER_HANDSHAKE_LEN,
};
use styx_runtime::{AppRuntime, RuntimeConfig, RuntimeEngine};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[test]
fn snapshot_converts_torrent_status_correctly() {
    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);
    let app_snap = runtime.snapshot();
    assert_eq!(app_snap.totals.torrent_count, 0);
}

#[test]
fn apply_add_loads_plan_and_inserts_torrent() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);
    use styx_app::ControlCommand;
    let response = runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();
    assert!(matches!(
        response,
        styx_app::CommandResponse::TorrentAdded { .. }
    ));
    let snap = runtime.snapshot();
    assert_eq!(snap.totals.torrent_count, 1);
}

#[test]
fn apply_remove_removes_torrent() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let info_hash = {
        let bytes = std::fs::read(&torrent).unwrap();
        let meta = styx_proto::decode_torrent(&bytes).unwrap();
        styx_app::InfoHashHex::new(*meta.info_hash_v1.as_bytes())
    };

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    assert_eq!(runtime.snapshot().totals.torrent_count, 1);

    runtime.apply(ControlCommand::Remove { info_hash }).unwrap();
    assert_eq!(runtime.snapshot().totals.torrent_count, 0);
}

#[test]
fn apply_unknown_remove_returns_error() {
    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);
    use styx_app::ControlCommand;
    let result = runtime.apply(ControlCommand::Remove {
        info_hash: styx_app::InfoHashHex::new([0u8; 20]),
    });
    assert!(result.is_err());
}

#[test]
fn tick_drains_engine_events() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    let events = runtime.tick();
    assert!(
        !events.is_empty(),
        "tick should return events from drained engine"
    );
    assert!(events
        .iter()
        .any(|e| matches!(e, styx_app::AppEvent::TorrentAdded { .. })));
}

#[test]
fn snapshot_accumulates_speed_samples_across_ticks() {
    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    let snap = runtime.snapshot();
    assert_eq!(snap.speed.len(), 0);

    runtime.tick();
    let snap = runtime.snapshot();
    assert_eq!(snap.speed.len(), 1);

    runtime.tick();
    let snap = runtime.snapshot();
    assert_eq!(snap.speed.len(), 2);
}

#[test]
fn apply_pause_and_resume_routes_through_engine() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let info_hash = {
        let bytes = std::fs::read(&torrent).unwrap();
        let meta = styx_proto::decode_torrent(&bytes).unwrap();
        styx_app::InfoHashHex::new(*meta.info_hash_v1.as_bytes())
    };

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    runtime.apply(ControlCommand::Pause { info_hash }).unwrap();
    assert_eq!(
        runtime.snapshot().torrents[0].status,
        styx_app::TorrentStatus::Paused
    );

    runtime.apply(ControlCommand::Resume { info_hash }).unwrap();
    assert_eq!(
        runtime.snapshot().torrents[0].status,
        styx_app::TorrentStatus::Downloading
    );
}

#[test]
fn tick_resolves_torrent_name_for_added_event() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    let events = runtime.tick();
    let added = events
        .iter()
        .find_map(|e| {
            if let styx_app::AppEvent::TorrentAdded { name, .. } = e {
                Some(name.as_str())
            } else {
                None
            }
        })
        .unwrap_or("");
    assert_eq!(added, "file.bin");
}

#[test]
fn tick_accumulates_logs_from_state_changed_events() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    runtime.tick();
    let snap = runtime.snapshot();
    assert!(
        !snap.logs.is_empty(),
        "tick should produce logs from events"
    );
    assert!(snap.logs.iter().any(|l| l.message.contains("added")));
}

#[test]
fn tick_logs_accumulate_across_multiple_ticks() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent.clone(),
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();
    let info_hash = {
        let bytes = std::fs::read(&torrent).unwrap();
        let meta = styx_proto::decode_torrent(&bytes).unwrap();
        styx_app::InfoHashHex::new(*meta.info_hash_v1.as_bytes())
    };

    runtime.tick();
    let first_logs = runtime.snapshot().logs.len();
    assert!(
        first_logs > 0,
        "add + auto-start produce at least one log entry"
    );

    runtime.apply(ControlCommand::Remove { info_hash }).unwrap();
    runtime.tick();
    assert!(
        runtime.snapshot().logs.len() > first_logs,
        "remove produces additional log entries"
    );
}

#[tokio::test]
async fn app_runtime_completes_added_torrent_from_tracker_peer() {
    let temp = tempfile::tempdir().unwrap();
    let piece_bytes = Bytes::from_static(b"abcd");
    let peer = serve_peer(piece_bytes.clone()).await;
    let tracker = serve_tracker(vec![peer]).await;
    let torrent = temp.path().join("peer.torrent");
    std::fs::write(&torrent, torrent_bytes_with_announce(tracker.as_str())).unwrap();

    let config = RuntimeConfig {
        source_timeout: std::time::Duration::from_secs(2),
        snapshot_interval: std::time::Duration::from_millis(10),
        ..RuntimeConfig::default()
    };
    let engine = RuntimeEngine::new(config).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    let mut completed = false;
    for _ in 0..100 {
        let events = runtime.tick();
        completed |= events
            .iter()
            .any(|event| matches!(event, styx_app::AppEvent::TorrentCompleted { .. }));
        if completed {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert_eq!(
        runtime.snapshot().torrents[0].status,
        styx_app::TorrentStatus::Seeding
    );
}

#[tokio::test]
async fn app_runtime_recovers_when_first_peer_disconnects_before_piece() {
    let temp = tempfile::tempdir().unwrap();
    let piece_bytes = Bytes::from_static(b"abcd");
    let bad_peer = serve_disconnect_after_request_peer().await;
    let good_peer = serve_peer_after_advertise_delay(
        piece_bytes.clone(),
        std::time::Duration::from_millis(150),
    )
    .await;
    let tracker = serve_tracker(vec![bad_peer, good_peer]).await;
    let torrent = temp.path().join("disconnect-recovery.torrent");
    std::fs::write(&torrent, torrent_bytes_with_announce(tracker.as_str())).unwrap();

    let config = RuntimeConfig {
        source_timeout: std::time::Duration::from_secs(2),
        snapshot_interval: std::time::Duration::from_millis(10),
        ..RuntimeConfig::default()
    };
    let engine = RuntimeEngine::new(config).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    tick_until_seeding(&mut runtime).await;

    assert!(
        runtime
            .snapshot()
            .logs
            .iter()
            .any(|line| line.message.contains("failed") || line.message.contains("disconnected")),
        "disconnect recovery should leave a failure/disconnect trail"
    );
}

#[tokio::test]
async fn app_runtime_quarantines_corrupt_peer_and_completes_from_web_seed() {
    let temp = tempfile::tempdir().unwrap();
    let corrupt_peer = serve_peer_with_payload(Bytes::from_static(b"wxyz")).await;
    let tracker = serve_tracker(vec![corrupt_peer]).await;
    let web_seed = serve_web_seed(Bytes::from_static(b"abcd")).await;
    let torrent = temp.path().join("corrupt-recovery.torrent");
    std::fs::write(
        &torrent,
        torrent_bytes_with_announce_and_web_seed(tracker.as_str(), web_seed.as_str()),
    )
    .unwrap();

    let config = RuntimeConfig {
        source_timeout: std::time::Duration::from_secs(3),
        snapshot_interval: std::time::Duration::from_millis(10),
        ..RuntimeConfig::default()
    };
    let engine = RuntimeEngine::new(config).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    tick_until_seeding(&mut runtime).await;

    let logs = runtime.snapshot().logs;
    assert!(
        logs.iter().any(|line| {
            line.message.contains("quarantined") || line.message.contains("failed")
        }),
        "corrupt peer recovery should leave a quarantine/source-failure trail; logs: {logs:?}"
    );
}

#[tokio::test]
async fn app_runtime_seeds_completed_torrent_to_interested_peer() {
    let temp = tempfile::tempdir().unwrap();
    let piece_bytes = Bytes::from_static(b"abcd");
    let mut leecher = serve_interested_leecher(piece_bytes.clone()).await;
    let tracker = serve_tracker_sequence(vec![Vec::new(), vec![leecher.addr]]).await;
    let web_seed = serve_web_seed(piece_bytes.clone()).await;
    let torrent = temp.path().join("seed-after-complete.torrent");
    std::fs::write(
        &torrent,
        torrent_bytes_with_announce_and_web_seed(tracker.as_str(), web_seed.as_str()),
    )
    .unwrap();
    let parsed_plan = styx_runtime::TorrentPlan::from_file(&torrent, temp.path()).unwrap();
    assert_eq!(parsed_plan.announce_urls, vec![tracker.clone()]);

    let config = RuntimeConfig {
        source_timeout: std::time::Duration::from_millis(120),
        snapshot_interval: std::time::Duration::from_millis(10),
        ..RuntimeConfig::default()
    };
    let engine = RuntimeEngine::new(config).unwrap();
    let mut runtime = AppRuntime::new(engine);

    runtime
        .apply(styx_app::ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    for _ in 0..200 {
        let _ = runtime.tick();
        match leecher.rx.try_recv() {
            Ok(received) => {
                assert_eq!(received, piece_bytes);
                return;
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                panic!("interested leecher task closed before receiving a piece");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let snapshot = runtime.snapshot();
    panic!(
        "interested leecher did not receive a seeded piece from AppRuntime; snapshot: {snapshot:?}"
    );
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

fn torrent_bytes_with_announce(announce: &str) -> Vec<u8> {
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
    info.insert(b"length".to_vec(), BencodeValue::Integer(4));
    info.insert(
        b"pieces".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(&Sha1::digest(b"abcd"))),
    );
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}

fn torrent_bytes_with_announce_and_web_seed(announce: &str, web_seed: &str) -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"announce".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(announce.as_bytes())),
    );
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
    info.insert(b"length".to_vec(), BencodeValue::Integer(4));
    info.insert(
        b"pieces".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(&Sha1::digest(b"abcd"))),
    );
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}

async fn serve_peer(piece_bytes: Bytes) -> SocketAddr {
    serve_peer_after_advertise_delay(piece_bytes, std::time::Duration::ZERO).await
}

async fn serve_peer_with_payload(piece_bytes: Bytes) -> SocketAddr {
    serve_peer_after_advertise_delay(piece_bytes, std::time::Duration::ZERO).await
}

async fn serve_peer_after_advertise_delay(
    piece_bytes: Bytes,
    delay: std::time::Duration,
) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
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
                peer_id: PeerId::new([9u8; 20]),
            },
        )
        .await
        .unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        write_message(
            &mut stream,
            &PeerMessage::Bitfield {
                bytes: Bytes::from_static(&[0x80]),
            },
        )
        .await
        .unwrap();
        write_message(&mut stream, &PeerMessage::Unchoke)
            .await
            .unwrap();

        loop {
            match styx_proto::read_message(&mut stream, styx_proto::DEFAULT_MAX_PEER_FRAME_LEN)
                .await
            {
                Ok(PeerMessage::Request {
                    index,
                    begin,
                    length,
                }) => {
                    assert_eq!(index, 0);
                    assert_eq!(begin, 0);
                    assert_eq!(length, piece_bytes.len() as u32);
                    write_message(
                        &mut stream,
                        &PeerMessage::Piece {
                            index,
                            begin,
                            block: piece_bytes,
                        },
                    )
                    .await
                    .unwrap();
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    addr
}

async fn serve_disconnect_after_request_peer() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        handshake_and_advertise(&mut stream).await;
        loop {
            match styx_proto::read_message(&mut stream, styx_proto::DEFAULT_MAX_PEER_FRAME_LEN)
                .await
            {
                Ok(PeerMessage::Request { .. }) => {
                    let _ = stream.shutdown().await;
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    addr
}

async fn handshake_and_advertise(stream: &mut tokio::net::TcpStream) {
    let mut handshake_bytes = [0u8; PEER_HANDSHAKE_LEN];
    stream.read_exact(&mut handshake_bytes).await.unwrap();
    let incoming = decode_handshake(&handshake_bytes).unwrap();
    write_handshake(
        stream,
        &Handshake {
            reserved: ExtensionBits::default(),
            info_hash: incoming.info_hash,
            peer_id: PeerId::new([9u8; 20]),
        },
    )
    .await
    .unwrap();
    write_message(
        stream,
        &PeerMessage::Bitfield {
            bytes: Bytes::from_static(&[0x80]),
        },
    )
    .await
    .unwrap();
    write_message(stream, &PeerMessage::Unchoke).await.unwrap();
}

async fn serve_tracker(peers: Vec<SocketAddr>) -> url::Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await.unwrap();
        let body = announce_response(&peers);
        stream.write_all(&http_response(&body)).await.unwrap();
    });
    url::Url::parse(&format!("http://{addr}/announce")).unwrap()
}

async fn serve_tracker_sequence(peer_batches: Vec<Vec<SocketAddr>>) -> url::Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for peers in peer_batches {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).await.unwrap();
            let body = announce_response(&peers);
            stream.write_all(&http_response(&body)).await.unwrap();
        }
    });
    url::Url::parse(&format!("http://{addr}/announce")).unwrap()
}

struct LeecherProbe {
    addr: SocketAddr,
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

async fn serve_web_seed(piece_bytes: Bytes) -> url::Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await.unwrap();
        stream
            .write_all(&http_response(&piece_bytes))
            .await
            .unwrap();
    });
    url::Url::parse(&format!("http://{addr}/file.bin")).unwrap()
}

async fn tick_until_seeding(runtime: &mut AppRuntime) {
    let mut completed = false;
    for _ in 0..150 {
        let events = runtime.tick();
        completed |= events
            .iter()
            .any(|event| matches!(event, styx_app::AppEvent::TorrentCompleted { .. }));
        if completed {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert_eq!(
        runtime.snapshot().torrents[0].status,
        styx_app::TorrentStatus::Seeding
    );
}

fn announce_response(peers: &[SocketAddr]) -> Vec<u8> {
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

fn compact_peers(peers: &[SocketAddr]) -> Vec<u8> {
    let mut out = Vec::new();
    for peer in peers {
        if let SocketAddr::V4(v4) = peer {
            out.extend_from_slice(&v4.ip().octets());
            out.extend_from_slice(&v4.port().to_be_bytes());
        }
    }
    out
}

fn http_response(body: &[u8]) -> Vec<u8> {
    let mut response =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    response.extend_from_slice(body);
    response
}
