use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use styx_proto::PeerId;
use tokio::sync::mpsc;

use crate::{
    download::download_all_pieces_from_web_seed, magnet::resolve_magnet_from_peers, MagnetAdd,
    MetadataFetchConfig, RuntimeCommand, RuntimeConfig, RuntimeEngine, RuntimeEvent,
    TorrentCommand, TorrentId, TorrentPlan, TorrentStatus, TorrentTask,
};

#[derive(Debug)]
pub(crate) enum BgEvent {
    Progress {
        id: TorrentId,
        verified_bytes: u64,
        #[allow(dead_code)]
        total_bytes: u64,
    },
    SourceFailed {
        id: TorrentId,
        source: String,
        reason: String,
    },
    Completed {
        id: TorrentId,
    },
    PeerDisconnected {
        id: TorrentId,
        addr: SocketAddr,
    },
    Runtime {
        event: RuntimeEvent,
    },
    Failed {
        id: TorrentId,
        reason: String,
    },
    MagnetMetadataResolved {
        id: TorrentId,
        plan: Box<TorrentPlan>,
    },
}

pub(crate) fn spawn_bg_magnet_resolution(
    id: TorrentId,
    add: MagnetAdd,
    tx: mpsc::UnboundedSender<BgEvent>,
    config: RuntimeConfig,
    peers: Vec<SocketAddr>,
) -> Option<tokio::task::JoinHandle<()>> {
    if tokio::runtime::Handle::try_current().is_err() {
        return None;
    }
    Some(tokio::spawn(async move {
        let metadata_config = MetadataFetchConfig {
            max_metadata_size: config.dht.metadata_size_limit,
            request_limit: config.dht.metadata_request_limit,
            timeout: config.source_timeout,
            ..MetadataFetchConfig::default()
        };
        let resolution = tokio::time::timeout(config.source_timeout, async {
            let magnet = styx_proto::parse_magnet_uri(&add.uri)
                .map_err(|err| crate::RuntimeError::Magnet(err.to_string()))?;
            let peers = if peers.is_empty() {
                magnet.exact_peers.clone()
            } else {
                peers
            };
            resolve_magnet_from_peers(
                add,
                magnet,
                peers,
                PeerId::new(rand::random()),
                metadata_config,
            )
            .await
        })
        .await;
        match resolution {
            Ok(Ok(resolved)) => {
                let _ = tx.send(BgEvent::MagnetMetadataResolved {
                    id,
                    plan: Box::new(resolved.plan),
                });
            }
            Ok(Err(err)) => {
                let _ = tx.send(BgEvent::Failed {
                    id,
                    reason: err.to_string(),
                });
            }
            Err(_) => {
                let _ = tx.send(BgEvent::Failed {
                    id,
                    reason: "magnet metadata resolution timed out".to_owned(),
                });
            }
        }
    }))
}

pub(crate) fn spawn_bg_download(
    plan: TorrentPlan,
    tx: mpsc::UnboundedSender<BgEvent>,
    config: RuntimeConfig,
) -> Option<tokio::task::JoinHandle<()>> {
    if tokio::runtime::Handle::try_current().is_err() {
        return None;
    }
    Some(tokio::spawn(async move {
        run_bg_download(plan, tx, config).await;
    }))
}

pub(crate) fn spawn_bg_seed(
    plan: TorrentPlan,
    tx: mpsc::UnboundedSender<BgEvent>,
    config: RuntimeConfig,
) -> Option<tokio::task::JoinHandle<()>> {
    if tokio::runtime::Handle::try_current().is_err() {
        return None;
    }
    Some(tokio::spawn(async move {
        run_bg_seed(plan, tx, config).await;
    }))
}

async fn run_bg_download(
    plan: TorrentPlan,
    tx: mpsc::UnboundedSender<BgEvent>,
    config: RuntimeConfig,
) {
    let id = plan.id;
    let total_size = plan.total_size;
    let config = match config.validate() {
        Ok(config) => config,
        Err(err) => {
            let _ = tx.send(BgEvent::Failed {
                id,
                reason: err.to_string(),
            });
            return;
        }
    };

    let _ = tx.send(BgEvent::Progress {
        id,
        verified_bytes: 0,
        total_bytes: total_size,
    });

    match run_peer_download(&plan, &tx, config.clone()).await {
        PeerAttempt::Completed => return,
        PeerAttempt::Unavailable => {}
        PeerAttempt::Failed(reason) if plan.web_seed_urls.is_empty() => {
            let _ = tx.send(BgEvent::Failed { id, reason });
            return;
        }
        PeerAttempt::Failed(_) => {}
    }

    run_web_seed_download(plan, tx, config.piece_timeout).await;
}

async fn run_bg_seed(plan: TorrentPlan, tx: mpsc::UnboundedSender<BgEvent>, config: RuntimeConfig) {
    let id = plan.id;
    let expected_pieces = plan.piece_count();
    let config = match config.validate() {
        Ok(config) => config,
        Err(err) => {
            let _ = tx.send(BgEvent::Failed {
                id,
                reason: err.to_string(),
            });
            return;
        }
    };
    let mut task = match TorrentTask::new_with_peers(plan, config.clone()) {
        Ok(task) => task,
        Err(err) => {
            let _ = tx.send(BgEvent::Failed {
                id,
                reason: err.to_string(),
            });
            return;
        }
    };
    match task.resume_verify().await {
        Ok(summary)
            if summary.verified == expected_pieces
                && summary.missing == 0
                && summary.failed == 0 => {}
        Ok(_) => {
            let _ = tx.send(BgEvent::Failed {
                id,
                reason: "seed data failed resume verification".to_owned(),
            });
            return;
        }
        Err(err) => {
            let _ = tx.send(BgEvent::Failed {
                id,
                reason: err.to_string(),
            });
            return;
        }
    }
    if let Err(err) = task
        .set_status_complete()
        .and_then(|_| task.start_seeding().map(drop))
    {
        let _ = tx.send(BgEvent::Failed {
            id,
            reason: err.to_string(),
        });
        return;
    }

    let tick_interval = config.snapshot_interval.min(Duration::from_millis(250));
    loop {
        let events = match task.discover_and_connect_peers().await {
            Ok(mut events) => match task.tick_seed_and_upload().await {
                Ok(tick_events) => {
                    events.extend(tick_events);
                    events
                }
                Err(err) => {
                    let _ = tx.send(BgEvent::Failed {
                        id,
                        reason: err.to_string(),
                    });
                    return;
                }
            },
            Err(err) => {
                let _ = tx.send(BgEvent::Failed {
                    id,
                    reason: err.to_string(),
                });
                return;
            }
        };
        for event in events {
            match event {
                RuntimeEvent::PeerDisconnected { addr, .. } => {
                    let _ = tx.send(BgEvent::PeerDisconnected { id, addr });
                }
                other => {
                    let _ = tx.send(BgEvent::Runtime { event: other });
                }
            }
        }
        tokio::time::sleep(tick_interval).await;
    }
}

enum PeerAttempt {
    Completed,
    Unavailable,
    Failed(String),
}

async fn run_peer_download(
    plan: &TorrentPlan,
    tx: &mpsc::UnboundedSender<BgEvent>,
    config: RuntimeConfig,
) -> PeerAttempt {
    if plan.announce_urls.is_empty() {
        return PeerAttempt::Unavailable;
    }

    let id = plan.id;
    let mut task = match TorrentTask::new_with_peers(plan.clone(), config.clone()) {
        Ok(task) => task,
        Err(err) => return PeerAttempt::Failed(err.to_string()),
    };
    if let Err(err) = task.apply(TorrentCommand::Start) {
        return PeerAttempt::Failed(err.to_string());
    }

    let tick_interval = config.snapshot_interval.min(Duration::from_millis(250));
    let mut last_activity = Instant::now();

    loop {
        let mut events = match task.discover_and_connect_peers().await {
            Ok(events) => events,
            Err(err) => return PeerAttempt::Failed(err.to_string()),
        };
        match task.tick_and_verify().await {
            Ok(tick_events) => events.extend(tick_events),
            Err(err) => return PeerAttempt::Failed(err.to_string()),
        }

        if emit_peer_events(id, &events, tx) {
            return PeerAttempt::Completed;
        }
        if peer_events_show_activity(&events) {
            last_activity = Instant::now();
        }
        if task.status() == TorrentStatus::Complete {
            let _ = tx.send(BgEvent::Completed { id });
            return PeerAttempt::Completed;
        }
        if last_activity.elapsed() >= config.source_timeout {
            return PeerAttempt::Failed("peer download made no progress before timeout".to_owned());
        }

        tokio::time::sleep(tick_interval).await;
    }
}

fn emit_peer_events(
    id: TorrentId,
    events: &[RuntimeEvent],
    tx: &mpsc::UnboundedSender<BgEvent>,
) -> bool {
    let mut completed = false;
    for event in events {
        match event {
            RuntimeEvent::ProgressUpdated {
                verified_bytes,
                total_bytes,
                ..
            } => {
                let _ = tx.send(BgEvent::Progress {
                    id,
                    verified_bytes: *verified_bytes,
                    total_bytes: *total_bytes,
                });
            }
            RuntimeEvent::SourceFailed { source, reason, .. } => {
                let _ = tx.send(BgEvent::SourceFailed {
                    id,
                    source: source.clone(),
                    reason: reason.clone(),
                });
            }
            RuntimeEvent::SourceQuarantined { source, .. } => {
                let _ = tx.send(BgEvent::SourceFailed {
                    id,
                    source: source.clone(),
                    reason: "source quarantined after failed verification".to_owned(),
                });
            }
            RuntimeEvent::TaskFailed { reason, .. } => {
                let _ = tx.send(BgEvent::Failed {
                    id,
                    reason: reason.clone(),
                });
            }
            RuntimeEvent::PeerDisconnected { addr, .. } => {
                let _ = tx.send(BgEvent::PeerDisconnected { id, addr: *addr });
            }
            RuntimeEvent::TaskCompleted { .. } => {
                let _ = tx.send(BgEvent::Completed { id });
                completed = true;
            }
            _ => {}
        }
    }
    completed
}

fn peer_events_show_activity(events: &[RuntimeEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::PeerConnected { .. }
                | RuntimeEvent::PieceVerified { .. }
                | RuntimeEvent::ProgressUpdated { .. }
        )
    })
}

async fn run_web_seed_download(
    plan: TorrentPlan,
    tx: mpsc::UnboundedSender<BgEvent>,
    piece_timeout: Duration,
) {
    let id = plan.id;
    let client = reqwest::Client::new();

    for seed in &plan.web_seed_urls {
        match download_all_pieces_from_web_seed(&client, &plan, seed, piece_timeout).await {
            Ok(pieces) => {
                let mut engine = match RuntimeEngine::new(RuntimeConfig::default()) {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = tx.send(BgEvent::Failed {
                            id,
                            reason: e.to_string(),
                        });
                        return;
                    }
                };
                if let Err(e) = engine.apply(RuntimeCommand::AddPlan(Box::new(plan.clone()))) {
                    let _ = tx.send(BgEvent::Failed {
                        id,
                        reason: e.to_string(),
                    });
                    return;
                }
                if let Err(e) = engine.apply(RuntimeCommand::Torrent(id, TorrentCommand::Start)) {
                    let _ = tx.send(BgEvent::Failed {
                        id,
                        reason: e.to_string(),
                    });
                    return;
                }

                match engine
                    .complete_from_source_piece_bytes(id, seed.as_str(), pieces)
                    .await
                {
                    Ok(()) => {
                        let _ = tx.send(BgEvent::Completed { id });
                        return;
                    }
                    Err(e) => {
                        let _ = tx.send(BgEvent::SourceFailed {
                            id,
                            source: seed.to_string(),
                            reason: e.to_string(),
                        });
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(BgEvent::SourceFailed {
                    id,
                    source: seed.to_string(),
                    reason: e.to_string(),
                });
            }
        }
    }

    let _ = tx.send(BgEvent::Failed {
        id,
        reason: "all web seeds failed".to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, net::SocketAddr, path::Path};

    use bytes::Bytes;
    use sha1::{Digest, Sha1};
    use styx_disk::DiskPlan;
    use styx_proto::{
        encode, read_handshake, read_message, write_handshake, write_message, BencodeValue,
        ExtensionBits, FileMode, Handshake, InfoHashV1, PeerId, PeerMessage, TorrentInfo,
        TorrentMetainfo,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    use url::Url;

    use super::*;

    #[tokio::test]
    async fn t8_t1_background_download_completes_from_tracker_peer() {
        let temp = tempfile::tempdir().unwrap();
        let piece_bytes = Bytes::from_static(b"abcd");
        let peer = serve_peer(piece_bytes.clone()).await;
        let tracker = serve_tracker(vec![peer]).await;
        let plan = make_plan(temp.path(), Some(tracker), Vec::new(), piece_bytes.as_ref());
        let config = RuntimeConfig {
            source_timeout: Duration::from_secs(2),
            snapshot_interval: Duration::from_millis(10),
            ..RuntimeConfig::default()
        };
        let (tx, mut rx) = mpsc::unbounded_channel();

        tokio::time::timeout(Duration::from_secs(5), run_bg_download(plan, tx, config))
            .await
            .unwrap();

        let mut completed = false;
        while let Ok(event) = rx.try_recv() {
            completed |= matches!(event, BgEvent::Completed { .. });
        }
        assert!(completed, "peer-backed background download should complete");
    }

    #[tokio::test]
    async fn t8_t2_background_download_falls_back_to_web_seed_when_no_peers_arrive() {
        let temp = tempfile::tempdir().unwrap();
        let piece_bytes = Bytes::from_static(b"abcd");
        let tracker = serve_tracker(Vec::new()).await;
        let web_seed = serve_web_seed(piece_bytes.clone()).await;
        let plan = make_plan(
            temp.path(),
            Some(tracker),
            vec![web_seed],
            piece_bytes.as_ref(),
        );
        let config = RuntimeConfig {
            source_timeout: Duration::from_millis(50),
            snapshot_interval: Duration::from_millis(10),
            piece_timeout: Duration::from_secs(2),
            ..RuntimeConfig::default()
        };
        let (tx, mut rx) = mpsc::unbounded_channel();

        tokio::time::timeout(Duration::from_secs(5), run_bg_download(plan, tx, config))
            .await
            .unwrap();

        let mut completed = false;
        while let Ok(event) = rx.try_recv() {
            completed |= matches!(event, BgEvent::Completed { .. });
        }
        assert!(
            completed,
            "web seed fallback should complete after peer idle"
        );
    }

    async fn serve_peer(piece_bytes: Bytes) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let info_hash = test_info_hash();
            let _ = read_handshake(&mut stream, info_hash).await.unwrap();
            write_handshake(
                &mut stream,
                &Handshake {
                    reserved: ExtensionBits::default(),
                    info_hash,
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
                match read_message(&mut stream, styx_proto::DEFAULT_MAX_PEER_FRAME_LEN).await {
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

    async fn serve_tracker(peers: Vec<SocketAddr>) -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).await.unwrap();
            let body = announce_response(&peers);
            stream.write_all(&http_response(&body)).await.unwrap();
        });
        Url::parse(&format!("http://{addr}/announce")).unwrap()
    }

    async fn serve_web_seed(piece_bytes: Bytes) -> Url {
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
        Url::parse(&format!("http://{addr}/file.bin")).unwrap()
    }

    fn make_plan(
        root: &Path,
        announce: Option<Url>,
        web_seed_urls: Vec<Url>,
        piece_bytes: &[u8],
    ) -> TorrentPlan {
        let info_hash = test_info_hash();
        let piece_hash: [u8; 20] = Sha1::digest(piece_bytes).into();
        let announce_bytes = announce
            .as_ref()
            .map(|url| Bytes::from(url.as_str().to_owned()));
        let metainfo = TorrentMetainfo {
            announce: announce_bytes,
            announce_list: Vec::new(),
            url_list: web_seed_urls
                .iter()
                .map(|url| Bytes::from(url.as_str().to_owned()))
                .collect(),
            info: TorrentInfo {
                name: Bytes::from_static(b"file.bin"),
                piece_length: piece_bytes.len() as u64,
                pieces: Some(Bytes::copy_from_slice(&piece_hash)),
                mode: FileMode::Single {
                    length: piece_bytes.len() as u64,
                },
                file_tree: None,
                meta_version: None,
                private: false,
            },
            info_hash_v1: info_hash,
            info_hash_v2: None,
            piece_layers: None,
            raw_info: Bytes::new(),
        };
        TorrentPlan {
            metainfo: metainfo.clone(),
            id: TorrentId::new(info_hash),
            info_hash,
            info_hash_v2: None,
            name: "file.bin".to_owned(),
            total_size: piece_bytes.len() as u64,
            announce_urls: announce.into_iter().collect(),
            web_seed_urls,
            disk_plan: DiskPlan::from_metainfo(&metainfo, root).unwrap(),
        }
    }

    fn test_info_hash() -> InfoHashV1 {
        InfoHashV1::new([7u8; 20])
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
}
