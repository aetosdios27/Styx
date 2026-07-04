use bytes::Bytes;
use styx_disk::{
    BlockSpec, DiskError, DiskPlan, PieceCompletion, PieceIndex, PieceManager, VerificationResult,
};

#[derive(Debug)]
pub struct PeerSessionDriver {
    pieces: PieceManager,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionOutcome {
    PieceVerified { piece: PieceIndex, bytes: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum SessionFailure {
    #[error("piece {piece:?} was corrupt")]
    CorruptPiece { piece: PieceIndex },
    #[error(transparent)]
    Disk(#[from] DiskError),
}

impl PartialEq for SessionFailure {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (
                Self::CorruptPiece { piece: left },
                Self::CorruptPiece { piece: right }
            ) if left == right
        )
    }
}

impl Eq for SessionFailure {}

impl PeerSessionDriver {
    #[must_use]
    pub fn new(plan: DiskPlan) -> Self {
        Self {
            pieces: PieceManager::new(plan),
        }
    }

    pub async fn accept_piece_blocks(
        &mut self,
        piece: PieceIndex,
        blocks: Vec<(BlockSpec, Bytes)>,
    ) -> Result<SessionOutcome, SessionFailure> {
        let expected_bytes = self.pieces.plan().piece_length(piece)? as u64;
        let mut complete = false;
        for (block, payload) in blocks {
            complete = matches!(
                self.pieces.accept_block(block, payload)?,
                PieceCompletion::Complete { .. }
            );
        }
        if !complete {
            return Err(SessionFailure::Disk(DiskError::MissingBlock));
        }
        match self.pieces.verify_and_commit_piece(piece).await? {
            VerificationResult::Verified { piece } => Ok(SessionOutcome::PieceVerified {
                piece,
                bytes: expected_bytes,
            }),
            VerificationResult::HashMismatch { piece } => {
                Err(SessionFailure::CorruptPiece { piece })
            }
        }
    }
}
