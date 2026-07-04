use std::ops::RangeInclusive;

use bytes::Bytes;
use styx_disk::PieceIndex;
use styx_proto::FileMode;

use crate::{RuntimeError, TorrentPlan};

pub fn piece_byte_range(
    plan: &TorrentPlan,
    piece: PieceIndex,
) -> Result<RangeInclusive<u64>, RuntimeError> {
    let length = u64::from(plan.piece_length(piece)?);
    let start = u64::from(piece.get()) * plan.metainfo.info.piece_length;
    let end = start + length - 1;
    Ok(start..=end)
}

pub fn web_seed_file_url(plan: &TorrentPlan, seed: &url::Url) -> Result<url::Url, RuntimeError> {
    let FileMode::Single { .. } = &plan.metainfo.info.mode else {
        return Err(RuntimeError::UnsupportedWebSeedLayout);
    };
    if !seed.path().ends_with('/') {
        return Ok(seed.clone());
    }
    seed.join(&plan.name)
        .map_err(|_| RuntimeError::InvalidConfig("failed to build web seed URL"))
}

pub fn validate_web_seed_piece_bytes(
    piece: PieceIndex,
    expected: u32,
    bytes: Bytes,
) -> Result<Bytes, RuntimeError> {
    if bytes.len() != expected as usize {
        return Err(RuntimeError::InvalidWebSeedLength {
            piece: piece.get(),
            expected: expected as usize,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}
