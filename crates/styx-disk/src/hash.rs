use sha1::{Digest as Sha1Digest, Sha1};
use sha2::Sha256;

use crate::{DiskError, DiskPlan, PieceIndex};

/// Verify v1 piece bytes against the SHA-1 hash stored in the disk plan.
///
/// # Errors
///
/// Returns [`DiskError::InvalidPieceIndex`] for an out-of-range piece and
/// [`DiskError::HashMismatch`] when the digest differs.
pub fn verify_v1_piece(plan: &DiskPlan, piece: PieceIndex, bytes: &[u8]) -> Result<(), DiskError> {
    let expected = plan.expected_v1_hash(piece)?;
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    if digest.as_slice() == expected {
        Ok(())
    } else {
        Err(DiskError::HashMismatch)
    }
}

/// Return the SHA-256 digest of one block.
#[must_use]
pub fn sha256_block_hash(block: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(block);
    hasher.finalize().into()
}

/// Return `SHA256(left || right)` for Merkle tree construction groundwork.
#[must_use]
pub fn merkle_parent_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use styx_proto::{FileMode, InfoHashV1, TorrentInfo, TorrentMetainfo};

    use super::*;
    use crate::{DiskError, DiskPlan, PieceIndex};

    #[test]
    fn verify_v1_piece_accepts_matching_sha1_digest() {
        let plan = plan_with_piece_hash(hex20("a9993e364706816aba3e25717850c26c9cd0d89d"));

        verify_v1_piece(&plan, PieceIndex::new(0), b"abc").unwrap();
    }

    #[test]
    fn verify_v1_piece_rejects_wrong_sha1_digest() {
        let plan = plan_with_piece_hash([0; 20]);

        let err = verify_v1_piece(&plan, PieceIndex::new(0), b"abc").unwrap_err();

        assert_eq!(err, DiskError::HashMismatch);
    }

    #[test]
    fn sha256_block_hash_returns_known_digest() {
        let digest = sha256_block_hash(b"abc");

        assert_eq!(
            digest,
            hex32("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }

    #[test]
    fn merkle_parent_hash_hashes_concatenated_children() {
        let left = sha256_block_hash(b"left");
        let right = sha256_block_hash(b"right");

        let parent = merkle_parent_hash(&left, &right);

        assert_eq!(
            parent,
            hex32("2a9870f5b7eb1cd732d95224cfea825a7b8772136cb497b20d2e3c612dfc90fe")
        );
    }

    fn plan_with_piece_hash(hash: [u8; 20]) -> DiskPlan {
        let meta = TorrentMetainfo {
            announce: None,
            announce_list: Vec::new(),
            url_list: Vec::new(),
            info: TorrentInfo {
                name: Bytes::from_static(b"file.bin"),
                piece_length: 16 * 1024,
                pieces: Bytes::copy_from_slice(&hash),
                private: false,
                mode: FileMode::Single { length: 3 },
            },
            info_hash_v1: InfoHashV1::new([0; 20]),
            raw_info: Bytes::new(),
        };
        DiskPlan::from_metainfo(&meta, "/tmp/styx").unwrap()
    }

    fn hex20(input: &str) -> [u8; 20] {
        let bytes = decode_hex(input);
        <[u8; 20]>::try_from(bytes.as_slice()).unwrap()
    }

    fn hex32(input: &str) -> [u8; 32] {
        let bytes = decode_hex(input);
        <[u8; 32]>::try_from(bytes.as_slice()).unwrap()
    }

    fn decode_hex(input: &str) -> Vec<u8> {
        input
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let text = std::str::from_utf8(pair).unwrap();
                u8::from_str_radix(text, 16).unwrap()
            })
            .collect()
    }
}
