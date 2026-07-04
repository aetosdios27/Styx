use std::ops::RangeInclusive;

use bytes::Bytes;
use styx_disk::PieceIndex;

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
