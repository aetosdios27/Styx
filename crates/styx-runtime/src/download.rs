use std::time::Duration;

use bytes::{Bytes, BytesMut};
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
            read_web_seed_body_bounded(response, piece, expected).await
        })
        .await
        .map_err(|_| RuntimeError::Timeout {
            stage: "downloading_web_seed_piece",
        })??;
        pieces.push(validate_web_seed_piece_bytes(piece, expected, bytes)?);
    }
    Ok(pieces)
}

pub(crate) async fn read_web_seed_body_bounded(
    mut response: reqwest::Response,
    piece: PieceIndex,
    expected: u32,
) -> Result<Bytes, RuntimeError> {
    let expected_usize = expected as usize;
    if let Some(length) = response.content_length() {
        if length > u64::from(expected) {
            return Err(RuntimeError::InvalidWebSeedLength {
                piece: piece.get(),
                expected: expected_usize,
                actual: usize::try_from(length).unwrap_or(usize::MAX),
            });
        }
    }
    let mut body = BytesMut::with_capacity(expected_usize);
    while let Some(chunk) = response.chunk().await? {
        let actual = body.len().saturating_add(chunk.len());
        if actual > expected_usize {
            return Err(RuntimeError::InvalidWebSeedLength {
                piece: piece.get(),
                expected: expected_usize,
                actual,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test]
    async fn bounded_web_seed_body_rejects_oversized_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\r\n")
                .await
                .unwrap();
            stream.write_all(&vec![b'x'; 1024 * 1024]).await.unwrap();
        });
        let response = reqwest::get(format!("http://{addr}/file.bin"))
            .await
            .unwrap();

        let error = read_web_seed_body_bounded(response, PieceIndex::new(0), 4)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            RuntimeError::InvalidWebSeedLength {
                expected: 4,
                actual: 1_048_576,
                ..
            }
        ));
    }
}
