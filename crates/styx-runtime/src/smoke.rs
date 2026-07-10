use std::net::SocketAddr;

use bytes::Bytes;
use reqwest::header::RANGE;
use styx_disk::{PieceManager, VerificationResult};
use styx_proto::{FileMode, PeerId, DEFAULT_MAX_PEER_FRAME_LEN};
use styx_tracker::HttpTrackerClient;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::{net::TcpStream, time::timeout};

use crate::{
    build_started_announce, download_piece_from_peer, load_torrent_plan, select_peer_candidates,
    PeerPieceRequest, RuntimeError, SmokeOutcome, SmokeRunConfig, TorrentSmokePlan,
};

pub async fn run_one_piece_smoke(config: SmokeRunConfig) -> Result<SmokeOutcome, RuntimeError> {
    config.validate()?;
    let plan = load_torrent_plan(&config.torrent_path, &config.destination, &config.limits)?;

    let mut candidates = Vec::new();
    if !plan.announce_urls.is_empty() {
        let tracker = HttpTrackerClient::new(config.limits.max_tracker_response_bytes);
        for announce_url in &plan.announce_urls {
            let request = build_started_announce(
                &plan,
                config.peer_id,
                config.listen_port,
                config.limits.numwant,
            );
            let response = timeout(
                config.limits.connect_timeout,
                tracker.announce(announce_url, &request),
            )
            .await
            .map_err(|_| RuntimeError::Timeout {
                stage: "announcing",
            })??;
            candidates.extend(select_peer_candidates(
                &response,
                config.limits.numwant as usize,
            ));
            if !candidates.is_empty() {
                break;
            }
        }
    }

    let mut last_error = None;
    for peer in candidates {
        match try_peer(&plan, config.peer_id, peer, &config).await {
            Ok(outcome) => return Ok(outcome),
            Err(error) => last_error = Some(error.to_string()),
        }
    }

    if !plan.web_seed_urls.is_empty() {
        match try_web_seeds(&plan, &config).await {
            Ok(outcome) => return Ok(outcome),
            Err(error) => last_error = Some(error.to_string()),
        }
    }

    if last_error.is_none() {
        return Err(RuntimeError::NoPeers);
    }

    Err(RuntimeError::AllPeersFailed {
        last_error: last_error.unwrap_or_else(|| "no peer attempts were made".to_owned()),
    })
}

async fn try_peer(
    plan: &TorrentSmokePlan,
    peer: PeerId,
    addr: SocketAddr,
    config: &SmokeRunConfig,
) -> Result<SmokeOutcome, RuntimeError> {
    let mut stream = timeout(config.limits.connect_timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| RuntimeError::Timeout {
            stage: "connecting_peer",
        })??;
    timeout(
        config.limits.peer_message_timeout,
        run_one_piece_smoke_with_stream(plan, peer, &mut stream),
    )
    .await
    .map_err(|_| RuntimeError::Timeout {
        stage: "downloading_piece",
    })?
}

async fn try_web_seeds(
    plan: &TorrentSmokePlan,
    config: &SmokeRunConfig,
) -> Result<SmokeOutcome, RuntimeError> {
    let client = reqwest::Client::new();
    let mut last_error = None;
    for seed in &plan.web_seed_urls {
        match timeout(
            config.limits.peer_message_timeout,
            download_piece_from_web_seed(&client, plan, seed),
        )
        .await
        .map_err(|_| RuntimeError::Timeout {
            stage: "downloading_web_seed_piece",
        })? {
            Ok(bytes) => return run_one_piece_smoke_with_web_seed_bytes(plan, bytes).await,
            Err(error) => last_error = Some(error.to_string()),
        }
    }

    Err(RuntimeError::AllPeersFailed {
        last_error: last_error.unwrap_or_else(|| "no web seed attempts were made".to_owned()),
    })
}

async fn download_piece_from_web_seed(
    client: &reqwest::Client,
    plan: &TorrentSmokePlan,
    seed: &url::Url,
) -> Result<Bytes, RuntimeError> {
    let url = web_seed_piece_url(plan, seed)?;
    let (start, end, expected_len) = target_piece_range(plan)?;
    let response = client
        .get(url)
        .header(RANGE, format!("bytes={start}-{end}"))
        .send()
        .await?
        .error_for_status()?;
    let expected_u32 = u32::try_from(expected_len)
        .map_err(|_| RuntimeError::InvalidConfig("web seed piece length exceeds u32"))?;
    let bytes =
        crate::download::read_web_seed_body_bounded(response, plan.target_piece, expected_u32)
            .await?;
    if bytes.len() != expected_len {
        return Err(RuntimeError::InvalidWebSeedLength {
            piece: plan.target_piece.get(),
            expected: expected_len,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}

fn web_seed_piece_url(plan: &TorrentSmokePlan, seed: &url::Url) -> Result<url::Url, RuntimeError> {
    let FileMode::Single { .. } = &plan.metainfo.info.mode else {
        return Err(RuntimeError::UnsupportedWebSeedLayout);
    };
    if !seed.path().ends_with('/') {
        return Ok(seed.clone());
    }
    let name = std::str::from_utf8(&plan.metainfo.info.name).map_err(|_| {
        RuntimeError::InvalidConfig("single-file torrent name must be utf-8 for web seed URL")
    })?;
    seed.join(name)
        .map_err(|_| RuntimeError::InvalidConfig("failed to build web seed URL"))
}

fn target_piece_range(plan: &TorrentSmokePlan) -> Result<(u64, u64, usize), RuntimeError> {
    let piece_len = plan.disk_plan.piece_length(plan.target_piece)?;
    let start = u64::from(plan.target_piece.get())
        .checked_mul(plan.metainfo.info.piece_length)
        .ok_or(RuntimeError::InvalidConfig("piece byte range overflow"))?;
    let end = start
        .checked_add(u64::from(piece_len))
        .and_then(|value| value.checked_sub(1))
        .ok_or(RuntimeError::InvalidConfig("piece byte range overflow"))?;
    Ok((start, end, piece_len as usize))
}

pub async fn run_one_piece_smoke_with_stream<S>(
    plan: &TorrentSmokePlan,
    local_peer_id: PeerId,
    stream: &mut S,
) -> Result<SmokeOutcome, RuntimeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut manager = PieceManager::new(plan.disk_plan.clone());
    let blocks = manager.next_blocks_for_piece(plan.target_piece)?;
    let downloaded = download_piece_from_peer(
        stream,
        PeerPieceRequest {
            info_hash: plan.info_hash,
            local_peer_id,
            target_piece: plan.target_piece,
            blocks,
            max_frame_len: DEFAULT_MAX_PEER_FRAME_LEN,
        },
    )
    .await?;

    let mut bytes = 0_u64;
    for (block, payload) in downloaded.blocks {
        bytes = bytes
            .checked_add(payload.len() as u64)
            .ok_or(RuntimeError::InvalidConfig(
                "downloaded byte count overflow",
            ))?;
        manager.accept_block(block, payload)?;
    }

    match manager.verify_and_commit_piece(plan.target_piece).await? {
        VerificationResult::Verified { piece } => Ok(SmokeOutcome::Verified {
            piece: piece.get(),
            bytes,
        }),
        VerificationResult::HashMismatch { piece } => {
            Err(RuntimeError::PieceHashMismatch { piece: piece.get() })
        }
    }
}

pub async fn run_one_piece_smoke_with_web_seed_bytes(
    plan: &TorrentSmokePlan,
    bytes: Bytes,
) -> Result<SmokeOutcome, RuntimeError> {
    let expected_len = plan.disk_plan.piece_length(plan.target_piece)? as usize;
    if bytes.len() != expected_len {
        return Err(RuntimeError::InvalidWebSeedLength {
            piece: plan.target_piece.get(),
            expected: expected_len,
            actual: bytes.len(),
        });
    }

    let mut manager = PieceManager::new(plan.disk_plan.clone());
    let blocks = manager.next_blocks_for_piece(plan.target_piece)?;
    let mut offset = 0_usize;
    for block in blocks {
        let len = block.length().get() as usize;
        let payload = bytes.slice(offset..offset + len);
        offset += len;
        manager.accept_block(block, payload)?;
    }

    match manager.verify_and_commit_piece(plan.target_piece).await? {
        VerificationResult::Verified { piece } => Ok(SmokeOutcome::Verified {
            piece: piece.get(),
            bytes: bytes.len() as u64,
        }),
        VerificationResult::HashMismatch { piece } => {
            Err(RuntimeError::PieceHashMismatch { piece: piece.get() })
        }
    }
}
