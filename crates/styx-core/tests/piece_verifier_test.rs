use std::collections::BTreeMap;

use sha1::{Digest, Sha1};
use styx_disk::{piece_root, sha256_block_hash};
use styx_proto::InfoHashV2;

fn test_pieces_root() -> InfoHashV2 {
    let mut bytes = [0u8; 32];
    bytes[0] = 1;
    InfoHashV2::new(bytes)
}

fn test_piece_layers() -> BTreeMap<InfoHashV2, Vec<[u8; 32]>> {
    let block0 = sha256_block_hash(&[0u8; 16384]);
    let block1 = sha256_block_hash(&[0u8; 16384]);
    let piece_hash = piece_root(&[block0, block1], 2);

    let mut layers = BTreeMap::new();
    layers.insert(test_pieces_root(), vec![piece_hash]);
    layers
}

fn test_piece_file_map() -> Vec<(InfoHashV2, u32)> {
    vec![(test_pieces_root(), 0)]
}

fn test_v1_hashes() -> Vec<[u8; 20]> {
    let hash = Sha1::digest([0u8; 32768]);
    let mut arr = [0u8; 20];
    arr.copy_from_slice(&hash);
    vec![arr]
}

fn make_v2_verifier() -> styx_core::piece_verifier::PieceVerifier {
    styx_core::piece_verifier::PieceVerifier::V2 {
        piece_layers: test_piece_layers(),
        piece_file_map: test_piece_file_map(),
        blocks_per_piece: 2,
    }
}

fn make_hybrid_verifier() -> styx_core::piece_verifier::PieceVerifier {
    styx_core::piece_verifier::PieceVerifier::Hybrid {
        expected_hashes_v1: test_v1_hashes(),
        piece_layers: test_piece_layers(),
        piece_file_map: test_piece_file_map(),
        blocks_per_piece: 2,
    }
}

#[test]
fn piece_verifier_v2_only() {
    let verifier = make_v2_verifier();
    let piece_data = vec![0u8; 32768];
    let result = verifier.verify_piece(0, &piece_data);
    assert!(result.is_ok());
}

#[test]
fn piece_verifier_hybrid_both_must_pass() {
    let verifier = make_hybrid_verifier();

    let data = vec![0u8; 32768];
    let result = verifier.verify_piece(0, &data);
    assert!(result.is_ok());

    let bad_data = vec![1u8; 32768];
    let result = verifier.verify_piece(0, &bad_data);
    assert!(result.is_err());
}
