use std::{collections::BTreeMap, net::SocketAddr, time::Duration};

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_app::{ControlCommand, InfoHashHex, TorrentRuntime};
use styx_proto::{
    decode_handshake, encode, read_message, write_handshake, write_message, BencodeValue,
    ExtensionBits, Handshake, PeerId, PeerMessage, PEER_HANDSHAKE_LEN,
};
use styx_runtime::{
    AppRuntime, RuntimeConfig, RuntimeEngine, TorrentCommand, TorrentPlan, TorrentTask,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot},
};

#[tokio::test]
async fn mock_leecher_downloads_single_block_from_styx_seed() {
    let piece = Bytes::from_static(b"abcd");
    let mut leecher = serve_leecher(vec![RequestSpec::new(0, piece.len() as u32)]).await;
    let tracker = serve_tracker(vec![leecher.addr]).await;
    let mut seed = completed_seed_task(std::slice::from_ref(&piece), tracker.as_str()).await;

    drive_seed_until_blocks(&mut seed.task, &mut leecher, 1, Duration::from_secs(2)).await;

    assert_eq!(leecher.blocks, vec![Bytes::from_static(b"abcd")]);
}

#[tokio::test]
async fn mock_leecher_downloads_multiple_blocks_from_styx_seed() {
    let piece = Bytes::from_static(b"abcdefghijkl");
    let mut leecher = serve_leecher(vec![
        RequestSpec::new(0, 4),
        RequestSpec::new(4, 4),
        RequestSpec::new(8, 4),
    ])
    .await;
    let tracker = serve_tracker(vec![leecher.addr]).await;
    let mut seed = completed_seed_task(&[piece], tracker.as_str()).await;

    drive_seed_until_blocks(&mut seed.task, &mut leecher, 3, Duration::from_secs(2)).await;

    assert_eq!(
        leecher.blocks,
        vec![
            Bytes::from_static(b"abcd"),
            Bytes::from_static(b"efgh"),
            Bytes::from_static(b"ijkl"),
        ]
    );
}

#[tokio::test]
async fn mock_leecher_cannot_download_after_torrent_paused() {
    let piece = Bytes::from_static(b"abcd");
    let mut leecher = serve_leecher(vec![RequestSpec::new(0, piece.len() as u32)]).await;
    let tracker = serve_tracker(vec![leecher.addr]).await;
    let mut seed = completed_seed_task(&[piece], tracker.as_str()).await;

    seed.task.discover_and_connect_peers().await.unwrap();
    seed.task.apply(TorrentCommand::Pause).unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
    while tokio::time::Instant::now() < deadline {
        let _ = seed.task.tick_seed_and_upload().await.unwrap();
        assert!(
            leecher.try_receive_block().is_none(),
            "paused seeder must not serve requested blocks"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn app_pause_stops_background_seed_worker_before_serving_new_peer() {
    let temp = tempfile::tempdir().unwrap();
    let piece = Bytes::from_static(b"abcd");
    let mut leecher = serve_leecher(vec![RequestSpec::new(0, piece.len() as u32)]).await;
    let (release_tracker, tracker) = serve_tracker_with_second_response_gate(leecher.addr).await;
    let web_seed = serve_web_seed(piece.clone()).await;
    let torrent = temp.path().join("seed-after-pause.torrent");
    std::fs::write(
        &torrent,
        torrent_with_announce_and_web_seed(&[piece], tracker.as_str(), web_seed.as_str()),
    )
    .unwrap();
    let info_hash = info_hash_hex(&torrent);
    let config = RuntimeConfig {
        snapshot_interval: Duration::from_millis(10),
        source_timeout: Duration::from_millis(150),
        ..RuntimeConfig::default()
    };
    let engine = RuntimeEngine::new(config).unwrap();
    let mut runtime = AppRuntime::new(engine);

    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();
    tick_app_until_seeding(&mut runtime).await;
    runtime.apply(ControlCommand::Pause { info_hash }).unwrap();
    release_tracker.send(()).unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while tokio::time::Instant::now() < deadline {
        let _ = runtime.tick();
        assert!(
            leecher.try_receive_block().is_none(),
            "paused AppRuntime seed worker must not serve newly discovered peers"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

struct SeedHarness {
    task: TorrentTask,
    _temp: tempfile::TempDir,
}

async fn completed_seed_task(chunks: &[Bytes], announce: &str) -> SeedHarness {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("seedable.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, torrent_with_announce(chunks, announce)).unwrap();
    let plan = TorrentPlan::from_file(&torrent, &destination).unwrap();
    let mut task = TorrentTask::new_with_peers(
        plan,
        RuntimeConfig {
            snapshot_interval: Duration::from_millis(10),
            ..RuntimeConfig::default()
        },
    )
    .unwrap();
    task.complete_from_piece_bytes(chunks.to_vec())
        .await
        .unwrap();
    SeedHarness { task, _temp: temp }
}

async fn drive_seed_until_blocks(
    task: &mut TorrentTask,
    leecher: &mut LeecherProbe,
    expected_blocks: usize,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        task.discover_and_connect_peers().await.unwrap();
        let _ = task.tick_seed_and_upload().await.unwrap();
        while leecher.try_receive_block().is_some() {}
        if leecher.blocks.len() >= expected_blocks {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!(
        "timed out waiting for {expected_blocks} seeded blocks; received {:?}",
        leecher.blocks
    );
}

fn torrent_with_announce(chunks: &[Bytes], announce: &str) -> Vec<u8> {
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
    info.insert(
        b"piece length".to_vec(),
        BencodeValue::Integer(chunks[0].len() as i64),
    );
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

fn torrent_with_announce_and_web_seed(chunks: &[Bytes], announce: &str, web_seed: &str) -> Vec<u8> {
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
    info.insert(
        b"piece length".to_vec(),
        BencodeValue::Integer(chunks[0].len() as i64),
    );
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

fn info_hash_hex(torrent: &std::path::Path) -> InfoHashHex {
    let bytes = std::fs::read(torrent).unwrap();
    let meta = styx_proto::decode_torrent(&bytes).unwrap();
    InfoHashHex::new(*meta.info_hash_v1.as_bytes())
}

#[derive(Clone, Copy)]
struct RequestSpec {
    begin: u32,
    length: u32,
}

impl RequestSpec {
    const fn new(begin: u32, length: u32) -> Self {
        Self { begin, length }
    }
}

struct LeecherProbe {
    addr: SocketAddr,
    blocks: Vec<Bytes>,
    rx: mpsc::UnboundedReceiver<Bytes>,
}

impl LeecherProbe {
    fn try_receive_block(&mut self) -> Option<Bytes> {
        match self.rx.try_recv() {
            Ok(block) => {
                self.blocks.push(block.clone());
                Some(block)
            }
            Err(_) => None,
        }
    }
}

async fn serve_leecher(requests: Vec<RequestSpec>) -> LeecherProbe {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
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
        assert_eq!(
            read_message(&mut stream, styx_proto::DEFAULT_MAX_PEER_FRAME_LEN)
                .await
                .unwrap(),
            PeerMessage::Unchoke
        );
        for request in requests {
            write_message(
                &mut stream,
                &PeerMessage::Request {
                    index: 0,
                    begin: request.begin,
                    length: request.length,
                },
            )
            .await
            .unwrap();
            let message = read_message(&mut stream, styx_proto::DEFAULT_MAX_PEER_FRAME_LEN)
                .await
                .unwrap();
            let PeerMessage::Piece { begin, block, .. } = message else {
                panic!("expected piece message, got {message:?}");
            };
            assert_eq!(begin, request.begin);
            tx.send(block).unwrap();
        }
    });
    LeecherProbe {
        addr,
        blocks: Vec::new(),
        rx,
    }
}

async fn serve_tracker(peers: Vec<SocketAddr>) -> url::Url {
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

async fn serve_tracker_with_second_response_gate(
    peer: SocketAddr,
) -> (oneshot::Sender<()>, url::Url) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = first.read(&mut buf).await.unwrap();
        let body = announce_response(&[]);
        first
            .write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
            )
            .await
            .unwrap();
        first.write_all(&body).await.unwrap();

        let (mut second, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = second.read(&mut buf).await.unwrap();
        let _ = rx.await;
        let body = announce_response(&[peer]);
        second
            .write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
            )
            .await
            .unwrap();
        second.write_all(&body).await.unwrap();
    });
    (
        tx,
        url::Url::parse(&format!("http://{addr}/announce")).unwrap(),
    )
}

async fn serve_web_seed(piece_bytes: Bytes) -> url::Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await.unwrap();
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                    piece_bytes.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(&piece_bytes).await.unwrap();
    });
    url::Url::parse(&format!("http://{addr}/file.bin")).unwrap()
}

async fn tick_app_until_seeding(runtime: &mut AppRuntime) {
    for _ in 0..200 {
        let _ = runtime.tick();
        if runtime.snapshot().torrents[0].status == styx_app::TorrentStatus::Seeding {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("AppRuntime did not reach seeding state");
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
