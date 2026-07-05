use std::time::Duration;

use bytes::Bytes;
use reqwest::header::RANGE;
use styx_disk::PieceIndex;
use tokio::time::timeout;

use crate::{
    piece_byte_range, validate_web_seed_piece_bytes, web_seed_file_url, DownloadOutcome,
    DownloadRunConfig, RuntimeCommand, RuntimeConfig, RuntimeEngine, RuntimeError, TorrentPlan,
};

pub async fn run_full_v1_download(
    config: DownloadRunConfig,
) -> Result<DownloadOutcome, RuntimeError> {
    config.validate()?;
    let plan = TorrentPlan::from_file(&config.torrent_path, &config.destination)?;
    if plan.web_seed_urls.is_empty() {
        return Err(RuntimeError::NoWebSeeds);
    }

    let id = plan.id;
    let total_size = plan.total_size;
    let piece_count = plan.piece_count();
    let mut engine = RuntimeEngine::new(RuntimeConfig::default())?;
    engine.apply(RuntimeCommand::AddPlan(Box::new(plan.clone())))?;

    let client = reqwest::Client::new();
    let mut last_error = None;
    for seed in &plan.web_seed_urls {
        match download_all_pieces_from_web_seed(
            &client,
            &plan,
            seed,
            config.limits.peer_message_timeout,
        )
        .await
        {
            Ok(pieces) => match engine
                .complete_from_source_piece_bytes(id, seed.as_str(), pieces)
                .await
            {
                Ok(()) => {
                    return Ok(DownloadOutcome::Complete {
                        pieces: piece_count,
                        bytes: total_size,
                    })
                }
                Err(error) => last_error = Some(error.to_string()),
            },
            Err(error) => last_error = Some(error.to_string()),
        }
    }

    Err(RuntimeError::AllPeersFailed {
        last_error: last_error.unwrap_or_else(|| "all sources failed".to_owned()),
    })
}

pub(crate) async fn download_all_pieces_from_web_seed(
    client: &reqwest::Client,
    plan: &TorrentPlan,
    seed: &url::Url,
    piece_timeout: Duration,
) -> Result<Vec<Bytes>, RuntimeError> {
    let url = web_seed_file_url(plan, seed)?;
    let mut pieces = Vec::with_capacity(plan.piece_count() as usize);
    for raw_piece in 0..plan.piece_count() {
        let piece = PieceIndex::new(raw_piece);
        let range = piece_byte_range(plan, piece)?;
        let expected = plan.piece_length(piece)?;
        let bytes = timeout(piece_timeout, async {
            let response = client
                .get(url.clone())
                .header(RANGE, format!("bytes={}-{}", range.start(), range.end()))
                .send()
                .await?
                .error_for_status()?;
            response.bytes().await
        })
        .await
        .map_err(|_| RuntimeError::Timeout {
            stage: "downloading_web_seed_piece",
        })??;
        pieces.push(validate_web_seed_piece_bytes(piece, expected, bytes)?);
    }
    Ok(pieces)
}
