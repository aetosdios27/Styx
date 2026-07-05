use std::collections::BTreeMap;

use sha1::{Digest, Sha1};
use styx_disk::verify_v2_piece_data;
use styx_proto::InfoHashV2;

/// Verification strategy for piece data.
pub enum PieceVerifier {
    /// v1-only: SHA-1 flat hash per piece
    V1 { expected_hashes: Vec<[u8; 20]> },
    /// v2-only: SHA-256 Merkle tree verification
    V2 {
        piece_layers: BTreeMap<InfoHashV2, Vec<[u8; 32]>>,
        piece_file_map: Vec<(InfoHashV2, u32)>,
        blocks_per_piece: u32,
    },
    /// Hybrid: BOTH v1 and v2 must verify
    Hybrid {
        expected_hashes_v1: Vec<[u8; 20]>,
        piece_layers: BTreeMap<InfoHashV2, Vec<[u8; 32]>>,
        piece_file_map: Vec<(InfoHashV2, u32)>,
        blocks_per_piece: u32,
    },
}

#[derive(Debug)]
pub enum VerificationError {
    HashMismatch,
    PieceOutOfRange,
    MissingPieceLayer,
}

impl PieceVerifier {
    pub fn verify_piece(
        &self,
        piece_index: u32,
        piece_data: &[u8],
    ) -> Result<(), VerificationError> {
        match self {
            PieceVerifier::V1 { expected_hashes } => {
                let expected = expected_hashes
                    .get(piece_index as usize)
                    .ok_or(VerificationError::PieceOutOfRange)?;
                if verify_v1_sha1(piece_data, expected) {
                    Ok(())
                } else {
                    Err(VerificationError::HashMismatch)
                }
            }
            PieceVerifier::V2 {
                piece_layers,
                piece_file_map,
                blocks_per_piece,
            } => self.verify_v2_piece(
                piece_index,
                piece_data,
                piece_layers,
                piece_file_map,
                *blocks_per_piece,
            ),
            PieceVerifier::Hybrid {
                expected_hashes_v1,
                piece_layers,
                piece_file_map,
                blocks_per_piece,
            } => {
                let v1_ok = expected_hashes_v1
                    .get(piece_index as usize)
                    .map(|h| verify_v1_sha1(piece_data, h))
                    .unwrap_or(false);
                if !v1_ok {
                    return Err(VerificationError::HashMismatch);
                }

                let v2_ok = self
                    .verify_v2_piece(
                        piece_index,
                        piece_data,
                        piece_layers,
                        piece_file_map,
                        *blocks_per_piece,
                    )
                    .is_ok();
                if !v2_ok {
                    return Err(VerificationError::HashMismatch);
                }

                Ok(())
            }
        }
    }

    fn verify_v2_piece(
        &self,
        piece_index: u32,
        piece_data: &[u8],
        piece_layers: &BTreeMap<InfoHashV2, Vec<[u8; 32]>>,
        piece_file_map: &[(InfoHashV2, u32)],
        blocks_per_piece: u32,
    ) -> Result<(), VerificationError> {
        let (pieces_root, local_idx) = piece_file_map
            .get(piece_index as usize)
            .ok_or(VerificationError::PieceOutOfRange)?;

        let layer_hashes = piece_layers
            .get(pieces_root)
            .ok_or(VerificationError::MissingPieceLayer)?;

        let expected_hash = layer_hashes
            .get(*local_idx as usize)
            .ok_or(VerificationError::PieceOutOfRange)?;

        if verify_v2_piece_data(piece_data, expected_hash, blocks_per_piece) {
            Ok(())
        } else {
            Err(VerificationError::HashMismatch)
        }
    }
}

fn verify_v1_sha1(data: &[u8], expected: &[u8; 20]) -> bool {
    let mut hasher = Sha1::new();
    hasher.update(data);
    let result = hasher.finalize();
    result.as_slice() == expected
}
