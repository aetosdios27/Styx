use std::collections::HashMap;

use bytes::Bytes;

use crate::{
    block_specs_for_piece, verify_v1_piece, BlockSpec, DiskError, DiskPlan, DiskStore, PieceBuffer,
    PieceCompletion, PieceIndex, ResumeSummary, VerificationResult,
};

/// Coordinates block assembly, verification, and durable piece commits.
#[derive(Debug)]
pub struct PieceManager {
    store: DiskStore,
    buffers: HashMap<u32, PieceBuffer>,
    have: Vec<bool>,
    pending: Vec<bool>,
}

impl PieceManager {
    /// Construct a manager from a disk plan.
    #[must_use]
    pub fn new(plan: DiskPlan) -> Self {
        let piece_count = plan.piece_count() as usize;
        Self {
            store: DiskStore::new(plan),
            buffers: HashMap::new(),
            have: vec![false; piece_count],
            pending: vec![false; piece_count],
        }
    }

    /// Return the manager's disk plan.
    #[must_use]
    pub fn plan(&self) -> &DiskPlan {
        self.store.plan()
    }

    /// Return the standard missing block specs for a piece.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError::InvalidPieceIndex`] when the piece is out of range.
    pub fn next_blocks_for_piece(&self, piece: PieceIndex) -> Result<Vec<BlockSpec>, DiskError> {
        if self.has_piece(piece) {
            return Ok(Vec::new());
        }
        block_specs_for_piece(piece, self.plan().piece_length(piece)?)
    }

    /// Accept one block payload into the selected piece buffer.
    ///
    /// # Errors
    ///
    /// Returns a [`DiskError`] for invalid pieces, invalid block bounds,
    /// duplicate/overlapping blocks, or wrong payload lengths.
    pub fn accept_block(
        &mut self,
        block: BlockSpec,
        payload: Bytes,
    ) -> Result<PieceCompletion, DiskError> {
        let piece = block.piece();
        let piece_length = self.plan().piece_length(piece)?;
        let block = BlockSpec::new(piece, block.offset(), block.length(), piece_length)?;
        let buffer = self
            .buffers
            .entry(piece.get())
            .or_insert(PieceBuffer::new(piece, piece_length)?);
        let completion = buffer.accept(block, payload)?;
        if matches!(completion, PieceCompletion::Complete { .. }) {
            set_flag(&mut self.pending, piece, true)?;
        }
        Ok(completion)
    }

    /// Verify a complete piece and commit it to disk.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError::MissingBlock`] when the piece buffer is incomplete,
    /// or an IO/validation error when commit fails.
    pub async fn verify_and_commit_piece(
        &mut self,
        piece: PieceIndex,
    ) -> Result<VerificationResult, DiskError> {
        self.plan().piece_length(piece)?;
        let Some(buffer) = self.buffers.get(&piece.get()) else {
            return Err(DiskError::MissingBlock);
        };
        let bytes = buffer.piece_bytes()?;

        if verify_v1_piece(self.plan(), piece, &bytes).is_err() {
            self.buffers.remove(&piece.get());
            set_flag(&mut self.pending, piece, false)?;
            return Ok(VerificationResult::HashMismatch { piece });
        }

        self.store.commit_piece(piece, bytes).await?;
        self.buffers.remove(&piece.get());
        set_flag(&mut self.pending, piece, false)?;
        set_flag(&mut self.have, piece, true)?;
        Ok(VerificationResult::Verified { piece })
    }

    /// Return whether a piece has been verified.
    #[must_use]
    pub fn has_piece(&self, piece: PieceIndex) -> bool {
        self.have
            .get(piece.get() as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Number of verified pieces.
    #[must_use]
    pub fn verified_piece_count(&self) -> u32 {
        self.have.iter().filter(|have| **have).count() as u32
    }

    /// Placeholder for resume verification, implemented in Task 7.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] for unexpected IO or validation failures. Missing
    /// and corrupted pieces are counted in the returned summary.
    pub async fn resume_verify(&mut self) -> Result<ResumeSummary, DiskError> {
        let mut summary = ResumeSummary::default();
        for raw_piece in 0..self.plan().piece_count() {
            let piece = PieceIndex::new(raw_piece);
            match self.store.read_piece(piece).await {
                Ok(bytes) => match verify_v1_piece(self.plan(), piece, &bytes) {
                    Ok(()) => {
                        set_flag(&mut self.have, piece, true)?;
                        summary.verified = summary
                            .verified
                            .checked_add(1)
                            .ok_or(DiskError::IntegerOverflow)?;
                    }
                    Err(DiskError::HashMismatch) => {
                        set_flag(&mut self.have, piece, false)?;
                        summary.failed = summary
                            .failed
                            .checked_add(1)
                            .ok_or(DiskError::IntegerOverflow)?;
                    }
                    Err(err) => return Err(err),
                },
                Err(DiskError::Io(err))
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    set_flag(&mut self.have, piece, false)?;
                    summary.missing = summary
                        .missing
                        .checked_add(1)
                        .ok_or(DiskError::IntegerOverflow)?;
                }
                Err(err) => return Err(err),
            }
        }
        Ok(summary)
    }
}

fn set_flag(flags: &mut [bool], piece: PieceIndex, value: bool) -> Result<(), DiskError> {
    let piece_count = u32::try_from(flags.len()).map_err(|_| DiskError::IntegerOverflow)?;
    let Some(flag) = flags.get_mut(piece.get() as usize) else {
        return Err(DiskError::InvalidPieceIndex {
            piece: piece.get(),
            piece_count,
        });
    };
    *flag = value;
    Ok(())
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use sha1::{Digest, Sha1};
    use styx_proto::{FileMode, InfoHashV1, TorrentInfo, TorrentMetainfo};

    use super::*;
    use crate::{BlockLength, BlockOffset, BlockSpec, PieceCompletion, PieceIndex};

    #[test]
    fn accept_block_returns_complete_after_all_blocks_arrive() {
        let mut manager = PieceManager::new(plan_for_bytes(b"abcdefgh"));
        let piece = PieceIndex::new(0);

        assert_eq!(
            manager
                .accept_block(block(piece, 0, 4, 8), Bytes::from_static(b"abcd"))
                .unwrap(),
            PieceCompletion::Incomplete
        );
        assert_eq!(
            manager
                .accept_block(block(piece, 4, 4, 8), Bytes::from_static(b"efgh"))
                .unwrap(),
            PieceCompletion::Complete { piece }
        );
    }

    #[test]
    fn next_blocks_for_piece_rejects_invalid_piece_index() {
        let manager = PieceManager::new(plan_for_bytes(b"abcdefgh"));

        let err = manager
            .next_blocks_for_piece(PieceIndex::new(1))
            .unwrap_err();

        assert_eq!(
            err,
            DiskError::InvalidPieceIndex {
                piece: 1,
                piece_count: 1,
            }
        );
    }

    #[tokio::test]
    async fn verify_and_commit_piece_writes_out_of_order_blocks() {
        let temp = tempfile::tempdir().unwrap();
        let mut manager = PieceManager::new(plan_for_root_and_bytes(temp.path(), b"abcdefgh"));
        let piece = PieceIndex::new(0);

        manager
            .accept_block(block(piece, 4, 4, 8), Bytes::from_static(b"efgh"))
            .unwrap();
        manager
            .accept_block(block(piece, 0, 4, 8), Bytes::from_static(b"abcd"))
            .unwrap();
        let result = manager.verify_and_commit_piece(piece).await.unwrap();

        assert_eq!(result, VerificationResult::Verified { piece });
        assert!(manager.has_piece(piece));
        assert_eq!(manager.verified_piece_count(), 1);
        assert_eq!(
            tokio::fs::read(temp.path().join("file.bin")).await.unwrap(),
            b"abcdefgh"
        );
    }

    #[tokio::test]
    async fn verify_and_commit_piece_rejects_hash_mismatch_without_marking_have() {
        let temp = tempfile::tempdir().unwrap();
        let mut manager = PieceManager::new(plan_for_root_and_bytes(temp.path(), b"abcdefgh"));
        let piece = PieceIndex::new(0);

        manager
            .accept_block(block(piece, 0, 4, 8), Bytes::from_static(b"xxxx"))
            .unwrap();
        manager
            .accept_block(block(piece, 4, 4, 8), Bytes::from_static(b"yyyy"))
            .unwrap();
        let result = manager.verify_and_commit_piece(piece).await.unwrap();

        assert_eq!(result, VerificationResult::HashMismatch { piece });
        assert!(!manager.has_piece(piece));
        assert_eq!(manager.verified_piece_count(), 0);
        assert!(!temp.path().join("file.bin").exists());
    }

    #[tokio::test]
    async fn resume_verify_marks_existing_correct_pieces_as_verified() {
        let temp = tempfile::tempdir().unwrap();
        tokio::fs::write(temp.path().join("file.bin"), b"abcdefgh")
            .await
            .unwrap();
        let mut manager = PieceManager::new(plan_for_root_and_pieces(
            temp.path(),
            &[b"abcd".as_slice(), b"efgh".as_slice()],
            4,
        ));

        let summary = manager.resume_verify().await.unwrap();

        assert_eq!(
            summary,
            ResumeSummary {
                verified: 2,
                missing: 0,
                failed: 0,
            }
        );
        assert!(manager.has_piece(PieceIndex::new(0)));
        assert!(manager.has_piece(PieceIndex::new(1)));
    }

    #[tokio::test]
    async fn resume_verify_counts_corrupted_piece_as_failed() {
        let temp = tempfile::tempdir().unwrap();
        tokio::fs::write(temp.path().join("file.bin"), b"abcdxxxx")
            .await
            .unwrap();
        let mut manager = PieceManager::new(plan_for_root_and_pieces(
            temp.path(),
            &[b"abcd".as_slice(), b"efgh".as_slice()],
            4,
        ));

        let summary = manager.resume_verify().await.unwrap();

        assert_eq!(
            summary,
            ResumeSummary {
                verified: 1,
                missing: 0,
                failed: 1,
            }
        );
        assert!(manager.has_piece(PieceIndex::new(0)));
        assert!(!manager.has_piece(PieceIndex::new(1)));
    }

    #[tokio::test]
    async fn resume_verify_counts_missing_files_without_panic() {
        let temp = tempfile::tempdir().unwrap();
        let mut manager = PieceManager::new(plan_for_root_and_pieces(
            temp.path(),
            &[b"abcd".as_slice()],
            4,
        ));

        let summary = manager.resume_verify().await.unwrap();

        assert_eq!(
            summary,
            ResumeSummary {
                verified: 0,
                missing: 1,
                failed: 0,
            }
        );
    }

    #[tokio::test]
    async fn concurrent_block_submission_commits_piece_once_with_external_mutex() {
        let temp = tempfile::tempdir().unwrap();
        let manager = std::sync::Arc::new(tokio::sync::Mutex::new(PieceManager::new(
            plan_for_root_and_bytes(temp.path(), b"abcdefgh"),
        )));
        let piece = PieceIndex::new(0);

        let first_manager = std::sync::Arc::clone(&manager);
        let first = tokio::spawn(async move {
            first_manager
                .lock()
                .await
                .accept_block(block(piece, 0, 4, 8), Bytes::from_static(b"abcd"))
                .unwrap()
        });
        let second_manager = std::sync::Arc::clone(&manager);
        let second = tokio::spawn(async move {
            second_manager
                .lock()
                .await
                .accept_block(block(piece, 4, 4, 8), Bytes::from_static(b"efgh"))
                .unwrap()
        });

        let _ = first.await.unwrap();
        let _ = second.await.unwrap();
        let result = manager
            .lock()
            .await
            .verify_and_commit_piece(piece)
            .await
            .unwrap();

        assert_eq!(result, VerificationResult::Verified { piece });
        assert_eq!(
            tokio::fs::read(temp.path().join("file.bin")).await.unwrap(),
            b"abcdefgh"
        );
    }

    fn block(piece: PieceIndex, offset: u32, length: u32, piece_length: u32) -> BlockSpec {
        BlockSpec::new(
            piece,
            BlockOffset::new(offset),
            BlockLength::new(length).unwrap(),
            piece_length,
        )
        .unwrap()
    }

    fn plan_for_bytes(bytes: &[u8]) -> DiskPlan {
        plan_for_root_and_bytes(std::path::Path::new("/tmp/styx-manager"), bytes)
    }

    fn plan_for_root_and_bytes(root: &std::path::Path, bytes: &[u8]) -> DiskPlan {
        plan_for_root_and_pieces(root, &[bytes], bytes.len() as u64)
    }

    fn plan_for_root_and_pieces(
        root: &std::path::Path,
        pieces: &[&[u8]],
        piece_length: u64,
    ) -> DiskPlan {
        let mut hashes = Vec::with_capacity(pieces.len() * 20);
        let mut total_length = 0_u64;
        for piece in pieces {
            let digest: [u8; 20] = Sha1::digest(piece).into();
            hashes.extend_from_slice(&digest);
            total_length += piece.len() as u64;
        }
        let meta = TorrentMetainfo {
            announce: None,
            announce_list: Vec::new(),
            info: TorrentInfo {
                name: Bytes::from_static(b"file.bin"),
                piece_length,
                pieces: Bytes::from(hashes),
                private: false,
                mode: FileMode::Single {
                    length: total_length,
                },
            },
            info_hash_v1: InfoHashV1::new([0; 20]),
            raw_info: Bytes::new(),
        };
        DiskPlan::from_metainfo(&meta, root).unwrap()
    }
}
