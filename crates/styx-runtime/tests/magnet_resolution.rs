use std::collections::BTreeMap;
use std::time::Duration;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_app::{ControlCommand, TorrentRuntime};
use styx_dht::{CompactPeer, DhtMessage, DhtQuery, DhtResponse, DhtSocket, NodeId};
use styx_proto::{
    decode_metadata_message, encode, encode_extension_handshake, encode_metadata_message,
    read_handshake, read_message, write_handshake, write_message, BencodeValue, ExtensionBits,
    ExtensionHandshake, Handshake, InfoHashV1, MetadataMessage, PeerId, PeerMessage,
    DEFAULT_MAX_PEER_FRAME_LEN,
};
use styx_runtime::{
    resolve_magnet_from_exact_peers, spawn_dht_worker, AppRuntime, DhtRuntimeConfig, MagnetAdd,
    MetadataFetchConfig, RuntimeConfig, RuntimeError,
};
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

    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
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

#[tokio::test]
async fn magnet_resolution_skips_wrong_metadata_hash_and_uses_next_peer() {
    let expected = torrent_bytes_for_piece(b"abcdefgh");
    let info_hash = info_hash_from_torrent_bytes(&expected);
    let wrong = torrent_bytes_for_piece(b"zzzzzzzz");
    let bad_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bad_addr = bad_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = bad_listener.accept().await.unwrap();
        serve_metadata_peer(&mut stream, info_hash, Bytes::from(wrong), 9).await;
    });
    let good_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let good_addr = good_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = good_listener.accept().await.unwrap();
        serve_metadata_peer(&mut stream, info_hash, Bytes::from(expected), 9).await;
    });
    let temp = tempfile::tempdir().unwrap();

    let resolved = resolve_magnet_from_exact_peers(
        MagnetAdd {
            uri: format!(
                "magnet:?xt=urn:btih:{}&x.pe={bad_addr}&x.pe={good_addr}",
                hex(info_hash.as_bytes())
            ),
            destination: temp.path().join("downloads"),
        },
        PeerId::new([1; 20]),
        MetadataFetchConfig {
            timeout: Duration::from_secs(1),
            ..MetadataFetchConfig::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(resolved.plan.info_hash, info_hash);
}

#[tokio::test]
async fn app_runtime_add_magnet_resolves_metadata_from_exact_peer() {
    let metadata = torrent_bytes_for_piece(b"abcdefgh");
    let info_hash = info_hash_from_torrent_bytes(&metadata);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        serve_metadata_peer(&mut stream, info_hash, Bytes::from(metadata), 9).await;
    });
    let temp = tempfile::tempdir().unwrap();
    let magnet = format!(
        "magnet:?xt=urn:btih:{}&x.pe={addr}",
        hex(info_hash.as_bytes())
    );
    let config = RuntimeConfig {
        source_timeout: Duration::from_secs(1),
        ..RuntimeConfig::default()
    };
    let mut runtime = AppRuntime::new_with_config(config).unwrap();

    runtime
        .apply(ControlCommand::AddMagnet {
            uri: magnet,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    for _ in 0..50 {
        runtime.tick();
        if runtime.snapshot().totals.torrent_count == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    server.await.unwrap();
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.totals.torrent_count, 1);
    assert_eq!(
        snapshot.torrents[0].info_hash.as_bytes(),
        info_hash.as_bytes()
    );
}

#[tokio::test]
async fn magnet_without_trackers_resolves_metadata_through_local_dht_and_downloads_piece() {
    let piece = Bytes::from_static(b"abcdefgh");
    let metadata = torrent_bytes_for_piece(&piece);
    let info_hash = info_hash_from_torrent_bytes(&metadata);
    let metadata_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let metadata_addr = metadata_listener.local_addr().unwrap();
    let metadata_server = tokio::spawn(async move {
        let (mut stream, _) = metadata_listener.accept().await.unwrap();
        serve_metadata_peer(&mut stream, info_hash, Bytes::from(metadata), 9).await;
        let (mut stream, _) = metadata_listener.accept().await.unwrap();
        serve_piece_peer(&mut stream, info_hash, piece).await;
    });
    let dht = DhtSocket::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let dht_addr = dht.local_addr().unwrap();
    let dht_server = tokio::spawn(async move {
        for _ in 0..2 {
            let event = dht.poll_once().await.unwrap();
            let DhtMessage::Query {
                transaction_id,
                query,
            } = event.message
            else {
                panic!("expected DHT query");
            };
            let response = match query {
                DhtQuery::Ping { .. } => DhtResponse::Ping {
                    id: NodeId::new([7; 20]),
                },
                DhtQuery::GetPeers { .. } => DhtResponse::GetPeers {
                    id: NodeId::new([7; 20]),
                    token: Bytes::from_static(b"token"),
                    values: vec![CompactPeer::new(metadata_addr)],
                    nodes: Vec::new(),
                    nodes6: Vec::new(),
                    external_ip: None,
                },
                other => panic!("unexpected DHT query: {other:?}"),
            };
            dht.send_to(
                &DhtMessage::Response {
                    transaction_id,
                    response,
                },
                event.source,
            )
            .await
            .unwrap();
        }
    });
    let dht_config = DhtRuntimeConfig {
        enabled: true,
        bind: "127.0.0.1:0".parse().unwrap(),
        bootstrap_nodes: vec![dht_addr],
        query_timeout: Duration::from_secs(1),
        command_capacity: 256,
        metadata_size_limit: 64 * 1024,
        metadata_request_limit: 8,
        tick_interval: Duration::from_millis(5),
    };
    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (client, owner) = spawn_dht_worker(dht_config.clone(), events_tx)
        .await
        .unwrap();
    let mut runtime = AppRuntime::new_with_config(RuntimeConfig {
        source_timeout: Duration::from_secs(1),
        dht: dht_config,
        ..RuntimeConfig::default()
    })
    .unwrap();
    runtime.attach_dht_worker(client, events_rx).unwrap();
    let temp = tempfile::tempdir().unwrap();

    runtime
        .apply(ControlCommand::AddMagnet {
            uri: format!("magnet:?xt=urn:btih:{}", hex(info_hash.as_bytes())),
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    for _ in 0..300 {
        runtime.tick();
        if runtime
            .snapshot()
            .torrents
            .first()
            .is_some_and(|torrent| torrent.progress == 1.0)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    owner.shutdown().await.unwrap();
    dht_server.await.unwrap();
    metadata_server.await.unwrap();
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.totals.torrent_count, 1);
    assert_eq!(snapshot.torrents[0].progress, 1.0);
}

#[tokio::test]
async fn magnet_resolution_times_out_without_hanging_runtime() {
    let dht = DhtSocket::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let dht_addr = dht.local_addr().unwrap();
    tokio::spawn(async move {
        let ping = dht.poll_once().await.unwrap();
        let DhtMessage::Query { transaction_id, .. } = ping.message else {
            panic!("expected bootstrap ping");
        };
        dht.send_to(
            &DhtMessage::Response {
                transaction_id,
                response: DhtResponse::Ping {
                    id: NodeId::new([8; 20]),
                },
            },
            ping.source,
        )
        .await
        .unwrap();
        let _unanswered_get_peers = dht.poll_once().await.unwrap();
    });
    let dht_config = DhtRuntimeConfig {
        enabled: true,
        bind: "127.0.0.1:0".parse().unwrap(),
        bootstrap_nodes: vec![dht_addr],
        query_timeout: Duration::from_millis(30),
        tick_interval: Duration::from_millis(5),
        ..DhtRuntimeConfig::default()
    };
    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (client, owner) = spawn_dht_worker(dht_config.clone(), events_tx)
        .await
        .unwrap();
    let mut runtime = AppRuntime::new_with_config(RuntimeConfig {
        dht: dht_config,
        ..RuntimeConfig::default()
    })
    .unwrap();
    runtime.attach_dht_worker(client, events_rx).unwrap();
    let temp = tempfile::tempdir().unwrap();
    runtime
        .apply(ControlCommand::AddMagnet {
            uri: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".into(),
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    for _ in 0..100 {
        runtime.tick();
        if runtime.persistent_state().torrents[0].state
            == styx_runtime::PersistentTorrentState::Failed
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    owner.shutdown().await.unwrap();
    assert_eq!(
        runtime.persistent_state().torrents[0].state,
        styx_runtime::PersistentTorrentState::Failed
    );
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

async fn serve_piece_peer(stream: &mut TcpStream, info_hash: InfoHashV1, piece: Bytes) {
    read_handshake(stream, info_hash).await.unwrap();
    write_handshake(
        stream,
        &Handshake {
            reserved: ExtensionBits::default(),
            info_hash,
            peer_id: PeerId::new([3; 20]),
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
    loop {
        match read_message(stream, DEFAULT_MAX_PEER_FRAME_LEN).await {
            Ok(PeerMessage::Request {
                index,
                begin,
                length,
            }) => {
                assert_eq!((index, begin, length), (0, 0, piece.len() as u32));
                write_message(
                    stream,
                    &PeerMessage::Piece {
                        index,
                        begin,
                        block: piece,
                    },
                )
                .await
                .unwrap();
                break;
            }
            Ok(_) => {}
            Err(error) => panic!("piece peer failed before request: {error}"),
        }
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
