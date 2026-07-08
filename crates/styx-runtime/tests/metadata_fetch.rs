use std::collections::BTreeMap;
use std::time::Duration;

use bytes::Bytes;
use styx_proto::{
    decode_metadata_message, encode_extension_handshake, encode_metadata_message, read_handshake,
    read_message, write_handshake, write_message, ExtensionBits, ExtensionHandshake, Handshake,
    InfoHashV1, MetadataMessage, PeerId, PeerMessage, DEFAULT_MAX_PEER_FRAME_LEN,
};
use styx_runtime::{fetch_metadata_from_stream, MetadataFetchConfig, RuntimeError};

#[tokio::test]
async fn fetch_metadata_from_stream_downloads_all_bep9_pieces() {
    let info_hash = InfoHashV1::new([7; 20]);
    let metadata = Bytes::from(vec![b'a'; 20_000]);
    let (mut client, mut server) = tokio::io::duplex(128 * 1024);
    let server_metadata = metadata.clone();

    let server = tokio::spawn(async move {
        serve_metadata_peer(&mut server, info_hash, server_metadata, 7).await;
    });

    let fetched = fetch_metadata_from_stream(
        &mut client,
        info_hash,
        PeerId::new([1; 20]),
        MetadataFetchConfig {
            max_metadata_size: 64 * 1024,
            request_limit: 8,
            timeout: Duration::from_secs(1),
            max_frame_len: 64 * 1024,
        },
    )
    .await
    .unwrap();

    server.await.unwrap();
    assert_eq!(fetched, metadata);
}

#[tokio::test]
async fn fetch_metadata_from_stream_rejects_oversized_metadata() {
    let info_hash = InfoHashV1::new([8; 20]);
    let (mut client, mut server) = tokio::io::duplex(16 * 1024);

    let server = tokio::spawn(async move {
        read_handshake(&mut server, info_hash).await.unwrap();
        write_handshake(
            &mut server,
            &Handshake {
                reserved: ExtensionBits::default().with_extended(),
                info_hash,
                peer_id: PeerId::new([2; 20]),
            },
        )
        .await
        .unwrap();
        let _ = read_message(&mut server, DEFAULT_MAX_PEER_FRAME_LEN)
            .await
            .unwrap();
        let mut messages = BTreeMap::new();
        messages.insert("ut_metadata".to_owned(), 7);
        write_message(
            &mut server,
            &PeerMessage::Extended {
                extension_id: 0,
                payload: Bytes::from(encode_extension_handshake(&ExtensionHandshake {
                    messages,
                    metadata_size: Some(65_536),
                    ..ExtensionHandshake::default()
                })),
            },
        )
        .await
        .unwrap();
    });

    let err = fetch_metadata_from_stream(
        &mut client,
        info_hash,
        PeerId::new([1; 20]),
        MetadataFetchConfig {
            max_metadata_size: 32 * 1024,
            request_limit: 8,
            timeout: Duration::from_secs(1),
            max_frame_len: 64 * 1024,
        },
    )
    .await
    .unwrap_err();

    server.await.unwrap();
    assert!(matches!(err, RuntimeError::Metadata(_)));
}

async fn serve_metadata_peer(
    stream: &mut tokio::io::DuplexStream,
    info_hash: InfoHashV1,
    metadata: Bytes,
    remote_metadata_id: u8,
) {
    let handshake = read_handshake(stream, info_hash).await.unwrap();
    assert!(handshake.reserved.supports_extended());
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

    let client_handshake = read_message(stream, DEFAULT_MAX_PEER_FRAME_LEN)
        .await
        .unwrap();
    assert!(matches!(
        client_handshake,
        PeerMessage::Extended {
            extension_id: 0,
            ..
        }
    ));
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
            panic!("expected metadata request payload");
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
