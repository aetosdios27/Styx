#![allow(dead_code)]

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_disk::{
    BlockSpec, DiskError, DiskPlan, PieceCompletion, PieceIndex, PieceManager, ResumeSummary,
    VerificationResult,
};
use styx_proto::{FileMode, InfoHashV1, TorrentInfo, TorrentMetainfo};

/// Injects I/O failures into a real PieceManager for torture testing.
#[derive(Debug)]
pub struct CorruptDiskHarness {
    inner: PieceManager,
    fail_mode: DiskFailMode,
    poisoned: AtomicBool,
}

#[derive(Debug)]
pub enum DiskFailMode {
    /// Normal operations — passes through to real PieceManager.
    None,
    /// Return PermissionDenied on verification calls.
    PermissionDenied,
    /// Return NoSpace on verification calls.
    NoSpace,
    /// Return ReadOnlyFilesystem on verification calls.
    ReadOnly,
}

impl CorruptDiskHarness {
    #[must_use]
    pub fn new(plan: DiskPlan, fail_mode: DiskFailMode) -> Self {
        Self {
            inner: PieceManager::new(plan),
            fail_mode,
            poisoned: AtomicBool::new(false),
        }
    }

    #[must_use]
    pub fn plan(&self) -> &DiskPlan {
        self.inner.plan()
    }

    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::Relaxed)
    }

    pub fn accept_block(
        &mut self,
        block: BlockSpec,
        payload: Bytes,
    ) -> Result<PieceCompletion, DiskError> {
        self.inner.accept_block(block, payload)
    }

    pub async fn verify_and_commit_piece(
        &mut self,
        piece: PieceIndex,
    ) -> Result<VerificationResult, DiskError> {
        match &self.fail_mode {
            DiskFailMode::None => self.inner.verify_and_commit_piece(piece).await,
            DiskFailMode::PermissionDenied => {
                self.poisoned.store(true, Ordering::Relaxed);
                Err(DiskError::Io(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "mock: permission denied",
                )))
            }
            DiskFailMode::NoSpace => {
                self.poisoned.store(true, Ordering::Relaxed);
                Err(DiskError::Io(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "mock: no space left",
                )))
            }
            DiskFailMode::ReadOnly => {
                self.poisoned.store(true, Ordering::Relaxed);
                Err(DiskError::Io(io::Error::new(
                    io::ErrorKind::ReadOnlyFilesystem,
                    "mock: read-only filesystem",
                )))
            }
        }
    }

    pub async fn resume_verify(&mut self) -> Result<ResumeSummary, DiskError> {
        self.inner.resume_verify().await
    }

    #[must_use]
    pub fn has_piece(&self, piece: PieceIndex) -> bool {
        self.inner.has_piece(piece)
    }

    #[must_use]
    pub fn verified_piece_count(&self) -> u32 {
        self.inner.verified_piece_count()
    }
}

fn plan_for_root_and_bytes(root: &std::path::Path, bytes: &[u8]) -> DiskPlan {
    let digest: [u8; 20] = Sha1::digest(bytes).into();
    let meta = TorrentMetainfo {
        announce: None,
        announce_list: Vec::new(),
        url_list: Vec::new(),
        info: TorrentInfo {
            name: Bytes::from_static(b"file.bin"),
            piece_length: bytes.len() as u64,
            pieces: Some(Bytes::from(digest.to_vec())),
            private: false,
            mode: FileMode::Single {
                length: bytes.len() as u64,
            },
            meta_version: None,
            file_tree: None,
        },
        info_hash_v1: InfoHashV1::new([0; 20]),
        info_hash_v2: None,
        piece_layers: None,
        raw_info: Bytes::new(),
    };
    DiskPlan::from_metainfo(&meta, root).unwrap()
}

#[cfg(test)]
mod tests {
    use styx_disk::{BlockLength, BlockOffset, BlockSpec, PieceIndex};

    use super::*;

    fn small_plan() -> DiskPlan {
        let dir = tempfile::tempdir().unwrap();
        plan_for_root_and_bytes(dir.path(), &[0u8; 16384])
    }

    fn block(piece: u32, offset: u32, length: u32) -> BlockSpec {
        BlockSpec::new(
            PieceIndex::new(piece),
            BlockOffset::new(offset),
            BlockLength::new(length).unwrap(),
            16384,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn corrupt_disk_none_passes_normal_operations() {
        let mut harness = CorruptDiskHarness::new(small_plan(), DiskFailMode::None);
        let b = block(0, 0, 16384);
        assert!(matches!(
            harness
                .accept_block(b, Bytes::from(vec![0; 16384]))
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

    #[tokio::test]
    async fn corrupt_disk_permission_denied_fails_verify() {
        let mut harness = CorruptDiskHarness::new(small_plan(), DiskFailMode::PermissionDenied);
        let b = block(0, 0, 16384);
        harness
            .accept_block(b, Bytes::from(vec![0; 16384]))
            .unwrap();
        let err = harness
            .verify_and_commit_piece(PieceIndex::new(0))
            .await
            .unwrap_err();
        assert!(matches!(err, DiskError::Io(ref e) if e.kind() == io::ErrorKind::PermissionDenied));
        assert!(harness.is_poisoned());
    }

    #[tokio::test]
    async fn corrupt_disk_no_space_fails_verify() {
        let mut harness = CorruptDiskHarness::new(small_plan(), DiskFailMode::NoSpace);
        let b = block(0, 0, 16384);
        harness
            .accept_block(b, Bytes::from(vec![0; 16384]))
            .unwrap();
        let err = harness
            .verify_and_commit_piece(PieceIndex::new(0))
            .await
            .unwrap_err();
        assert!(matches!(err, DiskError::Io(ref e) if e.kind() == io::ErrorKind::StorageFull));
        assert!(harness.is_poisoned());
    }

    #[tokio::test]
    async fn corrupt_disk_readonly_fails_verify() {
        let mut harness = CorruptDiskHarness::new(small_plan(), DiskFailMode::ReadOnly);
        let b = block(0, 0, 16384);
        harness
            .accept_block(b, Bytes::from(vec![0; 16384]))
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
}
