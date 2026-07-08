use std::collections::BTreeMap;
use std::time::Duration;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_proto::{
    decode_metadata_message, encode, encode_extension_handshake, encode_metadata_message,
    read_handshake, read_message, write_handshake, write_message, BencodeValue, ExtensionBits,
    ExtensionHandshake, Handshake, InfoHashV1, MetadataMessage, PeerId, PeerMessage,
    DEFAULT_MAX_PEER_FRAME_LEN,
};
use styx_runtime::{resolve_magnet_from_exact_peers, MagnetAdd, MetadataFetchConfig, RuntimeError};
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn resolve_magnet_from_exact_peer_returns_verified_decentralized_plan() {
    let metadata = torrent_bytes_for_piece(b"abcdefgh");
    let info_hash = info_hash_from_torrent_bytes(&metadata);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_metadata = Bytes::from(metadata);
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        serve_metadata_peer(&mut stream, info_hash, server_metadata, 9).await;
    });
    let temp = tempfile::tempdir().unwrap();
    let magnet = format!(
        "magnet:?xt=urn:btih:{}&x.pe={addr}",
        hex(info_hash.as_bytes())
    );

    let resolved = resolve_magnet_from_exact_peers(
        MagnetAdd {
            uri: magnet,
            destination: temp.path().join("downloads"),
        },
        PeerId::new([1; 20]),
        MetadataFetchConfig {
            timeout: Duration::from_secs(1),
            max_frame_len: 64 * 1024,
            ..MetadataFetchConfig::default()
        },
    )
    .await
    .unwrap();

    server.await.unwrap();
    assert_eq!(resolved.plan.info_hash, info_hash);
    assert!(resolved.plan.announce_urls.is_empty());
    assert!(resolved.plan.web_seed_urls.is_empty());
}

#[tokio::test]
async fn resolve_magnet_rejects_metadata_with_wrong_info_hash() {
    let expected_metadata = torrent_bytes_for_piece(b"abcdefgh");
    let actual_info_hash = info_hash_from_torrent_bytes(&expected_metadata);
    let wrong_metadata = torrent_bytes_for_piece(b"zzzzzzzz");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        serve_metadata_peer(
            &mut stream,
            actual_info_hash,
            Bytes::from(wrong_metadata),
            9,
        )
        .await;
    });
    let temp = tempfile::tempdir().unwrap();
    let magnet = format!(
        "magnet:?xt=urn:btih:{}&x.pe={addr}",
        hex(actual_info_hash.as_bytes())
    );

    let err = resolve_magnet_from_exact_peers(
        MagnetAdd {
            uri: magnet,
            destination: temp.path().join("downloads"),
        },
        PeerId::new([1; 20]),
        MetadataFetchConfig {
            timeout: Duration::from_secs(1),
            max_frame_len: 64 * 1024,
            ..MetadataFetchConfig::default()
        },
    )
    .await
    .unwrap_err();

    server.await.unwrap();
    assert!(matches!(err, RuntimeError::Magnet(_)));
}

async fn serve_metadata_peer(
    stream: &mut TcpStream,
    info_hash: InfoHashV1,
    metadata: Bytes,
    remote_metadata_id: u8,
) {
    read_handshake(stream, info_hash).await.unwrap();
    write_handshake(
        stream,
        &Handshake {
            reserved: ExtensionBits::default().with_extended(),
            info_hash,
            peer_id: PeerId::new([2; 20]),
        },
    )
    .await
    .unwrap();
    let _ = read_message(stream, DEFAULT_MAX_PEER_FRAME_LEN)
        .await
        .unwrap();

    let mut messages = BTreeMap::new();
    messages.insert("ut_metadata".to_owned(), remote_metadata_id);
    write_message(
        stream,
        &PeerMessage::Extended {
            extension_id: 0,
            payload: Bytes::from(encode_extension_handshake(&ExtensionHandshake {
                messages,
                metadata_size: Some(metadata.len() as u64),
                ..ExtensionHandshake::default()
            })),
        },
    )
    .await
    .unwrap();

    let piece_count = styx_proto::metadata_piece_count(metadata.len() as u64).unwrap();
    for _ in 0..piece_count {
        let request = read_message(stream, DEFAULT_MAX_PEER_FRAME_LEN)
            .await
            .unwrap();
        let PeerMessage::Extended {
            extension_id,
            payload,
        } = request
        else {
            panic!("expected metadata request");
        };
        assert_eq!(extension_id, remote_metadata_id);
        let MetadataMessage::Request { piece } = decode_metadata_message(&payload).unwrap() else {
            panic!("expected metadata request");
        };
        let block_len = styx_proto::METADATA_BLOCK_LEN as usize;
        let start = piece as usize * block_len;
        let end = (start + block_len).min(metadata.len());
        write_message(
            stream,
            &PeerMessage::Extended {
                extension_id: 1,
                payload: Bytes::from(encode_metadata_message(&MetadataMessage::Data {
                    piece,
                    total_size: metadata.len() as u64,
                    payload: metadata.slice(start..end),
                })),
            },
        )
        .await
        .unwrap();
    }
}

fn torrent_bytes_for_piece(piece: &[u8]) -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(b"info".to_vec(), info_dict(piece));
    encode(&BencodeValue::Dict(top))
}

fn info_dict(piece_bytes: &[u8]) -> BencodeValue {
    let piece = Sha1::digest(piece_bytes);
    let mut info = BTreeMap::new();
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"file.bin")),
    );
    info.insert(b"piece length".to_vec(), BencodeValue::Integer(8));
    info.insert(
        b"pieces".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(&piece)),
    );
    info.insert(b"length".to_vec(), BencodeValue::Integer(8));
    BencodeValue::Dict(info)
}

fn info_hash_from_torrent_bytes(bytes: &[u8]) -> InfoHashV1 {
    styx_proto::decode_torrent(bytes).unwrap().info_hash_v1
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
