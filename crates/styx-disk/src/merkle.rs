use sha2::{Digest, Sha256};

pub const MERKLE_BLOCK_SIZE: u32 = 16384;

#[must_use]
pub fn zero_hash() -> [u8; 32] {
    [0u8; 32]
}

#[must_use]
pub fn sha256_block_hash(block: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(block);
    hasher.finalize().into()
}

#[must_use]
pub fn merkle_parent_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

#[must_use]
pub fn merkle_root(leaf_hashes: &[[u8; 32]]) -> [u8; 32] {
    if leaf_hashes.is_empty() {
        return zero_hash();
    }

    let n = leaf_hashes.len().next_power_of_two();
    let mut layer: Vec<[u8; 32]> = leaf_hashes.to_vec();
    layer.resize(n, zero_hash());

    while layer.len() > 1 {
        layer = layer
            .chunks_exact(2)
            .map(|pair| merkle_parent_hash(&pair[0], &pair[1]))
            .collect();
    }

    layer[0]
}

#[must_use]
pub fn piece_root(block_hashes: &[[u8; 32]], _blocks_per_piece: u32) -> [u8; 32] {
    merkle_root(block_hashes)
}

#[must_use]
pub fn verify_v2_piece_data(
    piece_data: &[u8],
    expected_piece_hash: &[u8; 32],
    blocks_per_piece: u32,
) -> bool {
    let num_blocks = (piece_data.len() as u32).div_ceil(MERKLE_BLOCK_SIZE);
    let mut block_hashes = Vec::with_capacity(num_blocks as usize);

    for i in 0..num_blocks {
        let start = (i * MERKLE_BLOCK_SIZE) as usize;
        let end = std::cmp::min(start + MERKLE_BLOCK_SIZE as usize, piece_data.len());
        block_hashes.push(sha256_block_hash(&piece_data[start..end]));
    }

    let computed = piece_root(&block_hashes, blocks_per_piece);
    computed == *expected_piece_hash
}

#[must_use]
pub fn verify_block_with_proof(
    block_data: &[u8],
    block_index: u32,
    proof: &[[u8; 32]],
    expected_piece_hash: &[u8; 32],
) -> bool {
    let mut hash = sha256_block_hash(block_data);
    let mut idx = block_index;

    for sibling in proof {
        if idx.is_multiple_of(2) {
            hash = merkle_parent_hash(&hash, sibling);
        } else {
            hash = merkle_parent_hash(sibling, &hash);
        }
        idx /= 2;
    }

    hash == *expected_piece_hash
}

#[must_use]
pub fn piece_layer_hashes(block_hashes: &[[u8; 32]], blocks_per_piece: u32) -> Vec<[u8; 32]> {
    if block_hashes.is_empty() {
        return Vec::new();
    }

    let n = block_hashes.len().next_power_of_two();
    let mut layer: Vec<[u8; 32]> = block_hashes.to_vec();
    layer.resize(n, zero_hash());

    let target_level = (blocks_per_piece as u64).trailing_zeros() as usize;

    let mut current_level = 0;
    while layer.len() > 1 && current_level < target_level {
        layer = layer
            .chunks_exact(2)
            .map(|pair| merkle_parent_hash(&pair[0], &pair[1]))
            .collect();
        current_level += 1;
    }

    let num_data_pieces = (block_hashes.len() as u32).div_ceil(blocks_per_piece);
    layer.truncate(num_data_pieces as usize);

    layer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_hash_is_all_zeros() {
        assert_eq!(zero_hash(), [0u8; 32]);
    }

    #[test]
    fn sha256_block_hash_known_digest() {
        assert_eq!(
            sha256_block_hash(b"abc"),
            hex32("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }

    #[test]
    fn merkle_parent_hash_concatenates_leaves() {
        let left = sha256_block_hash(b"left");
        let right = sha256_block_hash(b"right");

        assert_eq!(
            merkle_parent_hash(&left, &right),
            hex32("2a9870f5b7eb1cd732d95224cfea825a7b8772136cb497b20d2e3c612dfc90fe")
        );
    }

    #[test]
    fn merkle_root_single_leaf() {
        let leaf = sha256_block_hash(b"data");
        assert_eq!(merkle_root(&[leaf]), leaf);
    }

    #[test]
    fn merkle_root_two_leaves() {
        let a = sha256_block_hash(b"a");
        let b = sha256_block_hash(b"b");
        let expected = merkle_parent_hash(&a, &b);
        assert_eq!(merkle_root(&[a, b]), expected);
    }

    #[test]
    fn merkle_root_four_leaves() {
        let data = vec![0u8; 4 * 16384];
        let block_hashes: Vec<[u8; 32]> = (0..4)
            .map(|i| sha256_block_hash(&data[i * 16384..(i + 1) * 16384]))
            .collect();

        let root = merkle_root(&block_hashes);

        let expected = merkle_parent_hash(
            &merkle_parent_hash(&block_hashes[0], &block_hashes[1]),
            &merkle_parent_hash(&block_hashes[2], &block_hashes[3]),
        );
        assert_eq!(root, expected);
    }

    #[test]
    fn merkle_root_pads_to_power_of_two() {
        let a = sha256_block_hash(b"a");
        let b = sha256_block_hash(b"b");
        let c = sha256_block_hash(b"c");
        let z = zero_hash();

        let expected = merkle_parent_hash(&merkle_parent_hash(&a, &b), &merkle_parent_hash(&c, &z));
        assert_eq!(merkle_root(&[a, b, c]), expected);
    }

    #[test]
    fn merkle_root_empty_returns_zero_hash() {
        assert_eq!(merkle_root(&[]), zero_hash());
    }

    #[test]
    fn verify_v2_piece_data_happy_path() {
        let data = vec![0u8; 16384];
        let hash = sha256_block_hash(&data);
        assert!(verify_v2_piece_data(&data, &hash, 1));
    }

    #[test]
    fn verify_v2_piece_data_rejects_mismatch() {
        let data = vec![0u8; 16384];
        let wrong = zero_hash();
        assert!(!verify_v2_piece_data(&data, &wrong, 1));
    }

    #[test]
    fn verify_block_with_proof_single_block() {
        let data = b"hello world";
        let block_hash = sha256_block_hash(data);
        assert!(verify_block_with_proof(data, 0, &[], &block_hash));
    }

    #[test]
    fn verify_block_with_proof_two_blocks() {
        let left = b"block0";
        let right = b"block1";
        let h0 = sha256_block_hash(left);
        let h1 = sha256_block_hash(right);
        let parent = merkle_parent_hash(&h0, &h1);

        assert!(verify_block_with_proof(left, 0, &[h1], &parent));
        assert!(verify_block_with_proof(right, 1, &[h0], &parent));
    }

    #[test]
    fn piece_layer_hashes_returns_per_piece_hashes() {
        let data = vec![0u8; 4 * 16384];
        let block_hashes: Vec<[u8; 32]> = (0..4)
            .map(|i| sha256_block_hash(&data[i * 16384..(i + 1) * 16384]))
            .collect();

        let layers = piece_layer_hashes(&block_hashes, 2);

        assert_eq!(layers.len(), 2);
        let p0 = merkle_parent_hash(&block_hashes[0], &block_hashes[1]);
        let p1 = merkle_parent_hash(&block_hashes[2], &block_hashes[3]);
        assert_eq!(layers[0], p0);
        assert_eq!(layers[1], p1);
    }

    #[test]
    fn piece_layer_hashes_truncates_padding() {
        let data = vec![0u8; 3 * 16384];
        let block_hashes: Vec<[u8; 32]> = (0..3)
            .map(|i| sha256_block_hash(&data[i * 16384..(i + 1) * 16384]))
            .collect();

        let layers = piece_layer_hashes(&block_hashes, 2);
        assert_eq!(layers.len(), 2);
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
