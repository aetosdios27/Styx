use bytes::Bytes;
use styx_disk::{BlockSpec, PieceIndex, PieceManager, VerificationResult};

use crate::{
    RuntimeError, RuntimeEvent, TorrentCommand, TorrentId, TorrentPlan, TorrentSnapshot,
    TorrentStatus,
};

#[derive(Debug)]
pub struct TorrentTask {
    plan: TorrentPlan,
    pieces: PieceManager,
    status: TorrentStatus,
    verified_bytes: u64,
    downloaded_bytes: u64,
}

impl TorrentTask {
    #[must_use]
    pub fn new(plan: TorrentPlan) -> Self {
        let pieces = PieceManager::new(plan.disk_plan.clone());
        Self {
            plan,
            pieces,
            status: TorrentStatus::Checking,
            verified_bytes: 0,
            downloaded_bytes: 0,
        }
    }

    #[must_use]
    pub fn id(&self) -> TorrentId {
        self.plan.id
    }

    #[must_use]
    pub fn status(&self) -> TorrentStatus {
        self.status
    }

    pub fn apply(&mut self, command: TorrentCommand) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        match command {
            TorrentCommand::Start => self.transition(TorrentStatus::Discovering),
            TorrentCommand::Pause => self.transition(TorrentStatus::Paused),
            TorrentCommand::Resume => self.transition(TorrentStatus::Downloading),
            TorrentCommand::Cancel => self.transition(TorrentStatus::Cancelled),
            TorrentCommand::Tick => self.tick(),
        }
    }

    pub async fn accept_piece_blocks(
        &mut self,
        piece: PieceIndex,
        blocks: Vec<(BlockSpec, Bytes)>,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let piece_bytes = u64::from(self.plan.piece_length(piece)?);
        for (block, payload) in blocks {
            self.downloaded_bytes = self.downloaded_bytes.saturating_add(payload.len() as u64);
            self.pieces.accept_block(block, payload)?;
        }
        match self.pieces.verify_and_commit_piece(piece).await? {
            VerificationResult::Verified { piece } => {
                self.verified_bytes = self.verified_bytes.saturating_add(piece_bytes);
                Ok(vec![
                    RuntimeEvent::PieceVerified {
                        torrent: self.plan.id,
                        piece: piece.get(),
                        bytes: piece_bytes,
                    },
                    RuntimeEvent::ProgressUpdated {
                        torrent: self.plan.id,
                        verified_bytes: self.verified_bytes,
                        total_bytes: self.plan.total_size,
                    },
                ])
            }
            VerificationResult::HashMismatch { piece } => {
                Err(RuntimeError::PieceHashMismatch { piece: piece.get() })
            }
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> TorrentSnapshot {
        let mut snapshot =
            TorrentSnapshot::new(self.plan.id, self.plan.name.clone(), self.plan.total_size)
                .with_verified_bytes(self.verified_bytes)
                .with_downloaded_bytes(self.downloaded_bytes);
        snapshot.status = self.status;
        snapshot
    }

    fn tick(&mut self) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        match self.status {
            TorrentStatus::Discovering => self.transition(TorrentStatus::Downloading),
            TorrentStatus::Checking => self.transition(TorrentStatus::Discovering),
            TorrentStatus::Downloading | TorrentStatus::Paused => Ok(Vec::new()),
            TorrentStatus::Complete
            | TorrentStatus::Seeding
            | TorrentStatus::Failed
            | TorrentStatus::Cancelled => Err(RuntimeError::InvalidConfig(
                "illegal torrent state transition",
            )),
        }
    }

    fn transition(&mut self, to: TorrentStatus) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let from = self.status;
        if !is_legal_transition(from, to) {
            return Err(RuntimeError::InvalidConfig(
                "illegal torrent state transition",
            ));
        }
        self.status = to;
        Ok(vec![RuntimeEvent::StateChanged {
            torrent: self.plan.id,
            from,
            to,
        }])
    }
}

fn is_legal_transition(from: TorrentStatus, to: TorrentStatus) -> bool {
    matches!(
        (from, to),
        (TorrentStatus::Checking, TorrentStatus::Discovering)
            | (TorrentStatus::Checking, TorrentStatus::Cancelled)
            | (TorrentStatus::Discovering, TorrentStatus::Downloading)
            | (TorrentStatus::Discovering, TorrentStatus::Paused)
            | (TorrentStatus::Discovering, TorrentStatus::Cancelled)
            | (TorrentStatus::Downloading, TorrentStatus::Paused)
            | (TorrentStatus::Downloading, TorrentStatus::Complete)
            | (TorrentStatus::Downloading, TorrentStatus::Cancelled)
            | (TorrentStatus::Paused, TorrentStatus::Downloading)
            | (TorrentStatus::Paused, TorrentStatus::Cancelled)
            | (TorrentStatus::Complete, TorrentStatus::Seeding)
    )
}
