use std::io;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_disk::{
    BlockLength, BlockOffset, BlockSpec, DiskError, DiskPlan, PieceCompletion, PieceIndex,
    VerificationResult,
};
use styx_proto::{FileMode, InfoHashV1, TorrentInfo, TorrentMetainfo};

use crate::abuse::corrupt_disk::{CorruptDiskHarness, DiskFailMode};

fn plan_for_single_piece(data: &[u8]) -> (DiskPlan, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let digest: [u8; 20] = Sha1::digest(data).into();
    let meta = TorrentMetainfo {
        announce: None,
        announce_list: Vec::new(),
        url_list: Vec::new(),
        info: TorrentInfo {
            name: Bytes::from_static(b"file.bin"),
            piece_length: data.len() as u64,
            pieces: Some(Bytes::from(digest.to_vec())),
            private: false,
            mode: FileMode::Single {
                length: data.len() as u64,
            },
            meta_version: None,
            file_tree: None,
        },
        info_hash_v1: InfoHashV1::new([0; 20]),
        info_hash_v2: None,
        piece_layers: None,
        raw_info: Bytes::new(),
    };
    let plan = DiskPlan::from_metainfo(&meta, dir.path()).unwrap();
    (plan, dir)
}

fn block(piece: u32, offset: u32, length: u32, piece_length: u32) -> BlockSpec {
    BlockSpec::new(
        PieceIndex::new(piece),
        BlockOffset::new(offset),
        BlockLength::new(length).unwrap(),
        piece_length,
    )
    .unwrap()
}

#[tokio::test]
async fn disk_pressure_no_space_fails_commit_on_verify() {
    let (plan, _dir) = plan_for_single_piece(&[0u8; 16384]);
    let mut harness = CorruptDiskHarness::new(plan, DiskFailMode::NoSpace);

    let b = block(0, 0, 16384, 16384);
    harness
        .accept_block(b, Bytes::from(vec![0u8; 16384]))
        .unwrap();

    let err = harness
        .verify_and_commit_piece(PieceIndex::new(0))
        .await
        .unwrap_err();
    assert!(matches!(err, DiskError::Io(ref e) if e.kind() == io::ErrorKind::StorageFull));
    assert!(harness.is_poisoned());
}

#[tokio::test]
async fn disk_pressure_permission_denied_fails_commit_on_verify() {
    let (plan, _dir) = plan_for_single_piece(&[0u8; 16384]);
    let mut harness = CorruptDiskHarness::new(plan, DiskFailMode::PermissionDenied);

    let b = block(0, 0, 16384, 16384);
    harness
        .accept_block(b, Bytes::from(vec![0u8; 16384]))
        .unwrap();

    let err = harness
        .verify_and_commit_piece(PieceIndex::new(0))
        .await
        .unwrap_err();
    assert!(
        matches!(err, DiskError::Io(ref e) if e.kind() == io::ErrorKind::PermissionDenied)
    );
    assert!(harness.is_poisoned());
}

#[tokio::test]
async fn disk_pressure_readonly_fails_commit_on_verify() {
    let (plan, _dir) = plan_for_single_piece(&[0u8; 16384]);
    let mut harness = CorruptDiskHarness::new(plan, DiskFailMode::ReadOnly);

    let b = block(0, 0, 16384, 16384);
    harness
        .accept_block(b, Bytes::from(vec![0u8; 16384]))
        .unwrap();

    let err = harness
        .verify_and_commit_piece(PieceIndex::new(0))
        .await
        .unwrap_err();
    assert!(
        matches!(err, DiskError::Io(ref e) if e.kind() == io::ErrorKind::ReadOnlyFilesystem)
    );
    assert!(harness.is_poisoned());
}

#[tokio::test]
async fn disk_pressure_no_fail_mode_passes_through() {
    let (plan, _dir) = plan_for_single_piece(&[0u8; 16384]);
    let mut harness = CorruptDiskHarness::new(plan, DiskFailMode::None);

    let b = block(0, 0, 16384, 16384);
    assert!(matches!(
        harness
            .accept_block(b, Bytes::from(vec![0u8; 16384]))
            .unwrap(),
        PieceCompletion::Complete { .. }
    ));

    let result = harness
        .verify_and_commit_piece(PieceIndex::new(0))
        .await
        .unwrap();
    assert!(matches!(result, VerificationResult::Verified { .. }));
    assert!(harness.has_piece(PieceIndex::new(0)));
    assert!(!harness.is_poisoned());
}
