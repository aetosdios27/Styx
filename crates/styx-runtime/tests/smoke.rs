use std::collections::BTreeMap;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_proto::{
    encode, read_handshake, read_message, write_handshake, write_message, BencodeValue,
    ExtensionBits, Handshake, PeerId, PeerMessage, DEFAULT_MAX_PEER_FRAME_LEN,
};
use styx_runtime::{
    load_torrent_plan, run_one_piece_smoke, run_one_piece_smoke_with_stream, SmokeConfig,
    SmokeOutcome, SmokeRunConfig,
};
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn run_one_piece_smoke_with_stream_verifies_and_writes_piece() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(&torrent, single_piece_torrent()).unwrap();
    let plan = load_torrent_plan(&torrent, &destination, &SmokeConfig::default()).unwrap();
    let (mut client, mut server) = duplex(4096);
    let info_hash = plan.info_hash;

    let peer = tokio::spawn(async move {
        let local = read_handshake(&mut server, info_hash).await.unwrap();
        write_handshake(
            &mut server,
            &Handshake {
                reserved: ExtensionBits::default(),
                info_hash,
                peer_id: PeerId::new([7; 20]),
            },
        )
        .await
        .unwrap();
        assert_eq!(local.peer_id, PeerId::new([6; 20]));
        assert_eq!(
            read_message(&mut server, DEFAULT_MAX_PEER_FRAME_LEN)
                .await
                .unwrap(),
            PeerMessage::Interested
        );
        write_message(&mut server, &PeerMessage::Unchoke)
            .await
            .unwrap();
        assert_eq!(
            read_message(&mut server, DEFAULT_MAX_PEER_FRAME_LEN)
                .await
                .unwrap(),
            PeerMessage::Request {
                index: 0,
                begin: 0,
                length: 4,
            }
        );
        write_message(
            &mut server,
            &PeerMessage::Piece {
                index: 0,
                begin: 0,
                block: Bytes::from_static(b"abcd"),
            },
        )
        .await
        .unwrap();
    });

    let outcome = run_one_piece_smoke_with_stream(&plan, PeerId::new([6; 20]), &mut client)
        .await
        .unwrap();

    peer.await.unwrap();
    assert_eq!(outcome, SmokeOutcome::Verified { piece: 0, bytes: 4 });
    assert_eq!(
        tokio::fs::read(destination.join("file.bin")).await.unwrap(),
        b"abcd"
    );
}

#[tokio::test]
#[ignore = "binds a localhost TCP socket for HTTP web-seed simulation"]
async fn run_one_piece_smoke_uses_web_seed_when_no_tracker_exists() {
    let temp = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/", listener.local_addr().unwrap());
    let torrent = temp.path().join("webseed.torrent");
    let destination = temp.path().join("webseed-downloads");
    std::fs::write(&torrent, single_piece_webseed_torrent(&base_url)).unwrap();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 2048];
        let read = socket.read(&mut request).await.unwrap();
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(request.starts_with("GET /file.bin HTTP/1.1"));
        assert!(request.contains("range: bytes=0-3") || request.contains("Range: bytes=0-3"));
        socket
            .write_all(
                b"HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 0-3/4\r\n\r\nabcd",
            )
            .await
            .unwrap();
    });

    let outcome = run_one_piece_smoke(SmokeRunConfig::default_for_paths(&torrent, &destination))
        .await
        .unwrap();

    server.await.unwrap();
    assert_eq!(outcome, SmokeOutcome::Verified { piece: 0, bytes: 4 });
    assert_eq!(
        tokio::fs::read(destination.join("file.bin")).await.unwrap(),
        b"abcd"
    );
}

#[tokio::test]
async fn run_one_piece_smoke_with_web_seed_bytes_verifies_and_writes_piece() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("webseed.torrent");
    let destination = temp.path().join("webseed-downloads");
    std::fs::write(
        &torrent,
        single_piece_webseed_torrent("https://mirror.test/iso/"),
    )
    .unwrap();
    let plan = load_torrent_plan(&torrent, &destination, &SmokeConfig::default()).unwrap();

    let outcome =
        styx_runtime::run_one_piece_smoke_with_web_seed_bytes(&plan, Bytes::from_static(b"abcd"))
            .await
            .unwrap();

    assert_eq!(outcome, SmokeOutcome::Verified { piece: 0, bytes: 4 });
    assert_eq!(
        tokio::fs::read(destination.join("file.bin")).await.unwrap(),
        b"abcd"
    );
}

fn single_piece_torrent() -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"announce".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"http://tracker.test/announce")),
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

fn single_piece_webseed_torrent(base_url: &str) -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"url-list".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(base_url.as_bytes())),
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
