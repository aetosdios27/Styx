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

async fn serve_peer(piece_bytes: Bytes) -> SocketAddr {
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
