use bytes::Bytes;
use styx_disk::{
    block_specs_for_piece, BlockSpec, PieceIndex, PieceManager, ResumeSummary, VerificationResult,
};

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
    pub fn into_plan(self) -> TorrentPlan {
        self.plan
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

    pub async fn complete_from_piece_bytes(
        &mut self,
        pieces: Vec<Bytes>,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        if pieces.len() != self.plan.piece_count() as usize {
            return Err(RuntimeError::InvalidConfig("piece byte count mismatch"));
        }

        let mut events = Vec::new();
        if matches!(self.status, TorrentStatus::Checking) {
            events.extend(self.transition(TorrentStatus::Discovering)?);
        }
        if matches!(self.status, TorrentStatus::Discovering) {
            events.extend(self.transition(TorrentStatus::Downloading)?);
        }

        for (raw_piece, piece_bytes) in pieces.into_iter().enumerate() {
            let piece = PieceIndex::new(raw_piece as u32);
            let specs = block_specs_for_piece(piece, self.plan.piece_length(piece)?)?;
            let mut offset = 0_usize;
            let mut blocks = Vec::with_capacity(specs.len());
            for spec in specs {
                let end = offset + spec.length().get() as usize;
                let Some(slice) = piece_bytes.get(offset..end) else {
                    return Err(RuntimeError::InvalidWebSeedLength {
                        piece: piece.get(),
                        expected: self.plan.piece_length(piece)? as usize,
                        actual: piece_bytes.len(),
                    });
                };
                blocks.push((spec, Bytes::copy_from_slice(slice)));
                offset = end;
            }
            if offset != piece_bytes.len() {
                return Err(RuntimeError::InvalidWebSeedLength {
                    piece: piece.get(),
                    expected: self.plan.piece_length(piece)? as usize,
                    actual: piece_bytes.len(),
                });
            }
            if !self.pieces.has_piece(piece) {
                events.extend(self.accept_piece_blocks(piece, blocks).await?);
            }
        }

        if self.pieces.verified_piece_count() == self.plan.piece_count() {
            events.extend(self.transition(TorrentStatus::Complete)?);
            events.push(RuntimeEvent::TaskCompleted {
                torrent: self.plan.id,
            });
        }
        Ok(events)
    }

    pub fn mark_failed(&mut self, reason: impl Into<String>) -> Vec<RuntimeEvent> {
        self.status = TorrentStatus::Failed;
        vec![RuntimeEvent::TaskFailed {
            torrent: self.plan.id,
            reason: reason.into(),
        }]
    }

    pub fn set_verified_bytes(&mut self, bytes: u64) {
        self.verified_bytes = bytes.min(self.plan.total_size);
    }

    pub fn set_status_complete(&mut self) {
        self.status = TorrentStatus::Complete;
        self.verified_bytes = self.plan.total_size;
    }

    pub async fn resume_verify(&mut self) -> Result<ResumeSummary, RuntimeError> {
        let summary = self.pieces.resume_verify().await?;
        let mut verified_bytes = 0_u64;
        for raw_piece in 0..self.plan.piece_count() {
            let piece = PieceIndex::new(raw_piece);
            if self.pieces.has_piece(piece) {
                verified_bytes =
                    verified_bytes.saturating_add(u64::from(self.plan.piece_length(piece)?));
            }
        }
        self.verified_bytes = verified_bytes;
        Ok(summary)
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
