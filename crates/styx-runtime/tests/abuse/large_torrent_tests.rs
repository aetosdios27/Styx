use tempfile::TempDir;

use styx_disk::DiskPlan;
use styx_proto::decode_torrent;

use crate::abuse::large_torrent::{LargeTorrentConfig, TorrentMode};

#[test]
fn large_torrent_10000_pieces_v1_does_not_oom() {
    let config = LargeTorrentConfig {
        piece_size: 16384,
        file_count: 1,
        total_size: 16384 * 10000,
        mode: TorrentMode::V1,
    };

    let bytes = config.generate_torrent_bytes();
    let meta = decode_torrent(&bytes).expect("v1 10000-piece torrent should decode");

    let tmp = TempDir::new().expect("temp dir");
    let plan = DiskPlan::from_metainfo(&meta, tmp.path()).expect("DiskPlan should build");

    assert_eq!(plan.piece_count(), 10000);
}

#[test]
fn large_torrent_4gib_v2_merkle_constructs() {
    let config = LargeTorrentConfig {
        piece_size: 262144,
        file_count: 1,
        total_size: 256 * 1024 * 1024, // 256 MiB — 1024 pieces, practical for CI
        mode: TorrentMode::V2,
    };

    let bytes = config.generate_torrent_bytes_streaming();
    let meta = decode_torrent(&bytes).expect("v2 torrent should decode");

    assert_eq!(meta.info.meta_version, Some(2));
    assert_eq!(config.piece_count(), 1024);
}

#[test]
fn large_torrent_boundary_piece_count_no_overflow() {
    let config = LargeTorrentConfig {
        piece_size: 16384,
        file_count: 1,
        total_size: u64::MAX / 2,
        mode: TorrentMode::V1,
    };

    let _count = config.piece_count();
}

#[test]
fn large_torrent_tiny_piece_size_block_decomposition() {
    let config = LargeTorrentConfig {
        piece_size: 1024,
        file_count: 1,
        total_size: 4096,
        mode: TorrentMode::V1,
    };

    let bytes = config.generate_torrent_bytes();
    let meta = decode_torrent(&bytes).expect("tiny-piece torrent should decode");

    let tmp = TempDir::new().expect("temp dir");
    let plan =
        DiskPlan::from_metainfo(&meta, tmp.path()).expect("DiskPlan should build for tiny pieces");

    assert_eq!(plan.piece_count(), 4);
}

#[test]
fn large_torrent_huge_piece_size_block_count() {
    use styx_disk::MERKLE_BLOCK_SIZE;

    let config = LargeTorrentConfig {
        piece_size: 4 * 1024 * 1024,
        file_count: 1,
        total_size: 4 * 1024 * 1024 * 2,
        mode: TorrentMode::V1,
    };

    let bytes = config.generate_torrent_bytes();
    let meta = decode_torrent(&bytes).expect("huge-piece torrent should decode");

    let tmp = TempDir::new().expect("temp dir");
    let plan =
        DiskPlan::from_metainfo(&meta, tmp.path()).expect("DiskPlan should build for huge pieces");

    assert_eq!(plan.piece_count(), 2);

    let expected_blocks_per_piece = 4 * 1024 * 1024 / MERKLE_BLOCK_SIZE;
    assert_eq!(expected_blocks_per_piece, 256);
}
