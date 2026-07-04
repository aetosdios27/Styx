use std::collections::{BTreeMap, VecDeque};

use bytes::Bytes;
use styx_disk::{BlockSpec, PieceIndex, ResumeSummary};

use crate::{
    RollbackRecord, RuntimeCommand, RuntimeConfig, RuntimeError, RuntimeEvent, RuntimeSnapshot,
    SettingsPatch, TorrentCommand, TorrentId, TorrentPlan, TorrentTask,
};

#[derive(Debug)]
pub struct RuntimeEngine {
    config: RuntimeConfig,
    tasks: BTreeMap<TorrentId, TorrentTask>,
    events: VecDeque<RuntimeEvent>,
}

impl RuntimeEngine {
    pub fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        Ok(Self {
            config: config.validate()?,
            tasks: BTreeMap::new(),
            events: VecDeque::new(),
        })
    }

    pub fn has_torrent(&self, id: TorrentId) -> bool {
        self.tasks.contains_key(&id)
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn apply(&mut self, command: RuntimeCommand) -> Result<(), RuntimeError> {
        match command {
            RuntimeCommand::AddPlan(plan) => self.add_plan(*plan),
            RuntimeCommand::Torrent(id, command) => self.apply_torrent(id, command),
            RuntimeCommand::Remove(id) => {
                self.tasks
                    .remove(&id)
                    .ok_or(RuntimeError::InvalidConfig("unknown torrent"))?;
                Ok(())
            }
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            torrents: self.tasks.values().map(TorrentTask::snapshot).collect(),
            peers: Vec::new(),
            events: self.events.iter().cloned().collect(),
        }
    }

    pub fn drain_events(&mut self) -> Vec<RuntimeEvent> {
        self.events.drain(..).collect()
    }

    pub async fn accept_piece_blocks(
        &mut self,
        id: TorrentId,
        piece: PieceIndex,
        blocks: Vec<(BlockSpec, Bytes)>,
    ) -> Result<(), RuntimeError> {
        let task = self
            .tasks
            .get_mut(&id)
            .ok_or(RuntimeError::InvalidConfig("unknown torrent"))?;
        let events = task.accept_piece_blocks(piece, blocks).await?;
        for event in events {
            self.push_event(event);
        }
        Ok(())
    }

    pub async fn complete_from_piece_bytes(
        &mut self,
        id: TorrentId,
        pieces: Vec<Bytes>,
    ) -> Result<(), RuntimeError> {
        let task = self
            .tasks
            .get_mut(&id)
            .ok_or(RuntimeError::InvalidConfig("unknown torrent"))?;
        let events = task.complete_from_piece_bytes(pieces).await?;
        for event in events {
            self.push_event(event);
        }
        Ok(())
    }

    pub async fn complete_from_source_piece_bytes(
        &mut self,
        id: TorrentId,
        source: impl Into<String>,
        pieces: Vec<Bytes>,
    ) -> Result<(), RuntimeError> {
        let source = source.into();
        match self.complete_from_piece_bytes(id, pieces).await {
            Ok(()) => Ok(()),
            Err(RuntimeError::PieceHashMismatch { piece }) => {
                self.push_event(RuntimeEvent::SourceQuarantined {
                    torrent: id,
                    source: source.clone(),
                });
                Err(RuntimeError::SourceFailed {
                    source_id: source,
                    scope: crate::FailureScope::SourceLocal,
                    retry: crate::RetryClass::Quarantine,
                    reason: format!("piece {piece} failed hash verification"),
                })
            }
            Err(err) => {
                self.push_event(RuntimeEvent::SourceFailed {
                    torrent: id,
                    source: source.clone(),
                    reason: err.to_string(),
                });
                Err(RuntimeError::SourceFailed {
                    source_id: source,
                    scope: crate::FailureScope::SourceLocal,
                    retry: crate::RetryClass::Retryable,
                    reason: err.to_string(),
                })
            }
        }
    }

    pub async fn complete_from_sources(
        &mut self,
        id: TorrentId,
        sources: Vec<(&str, Vec<Bytes>)>,
    ) -> Result<(), RuntimeError> {
        let mut last_error = "all sources failed".to_owned();
        for (source, pieces) in sources {
            match self
                .complete_from_source_piece_bytes(id, source.to_owned(), pieces)
                .await
            {
                Ok(()) => return Ok(()),
                Err(err) => last_error = err.to_string(),
            }
        }
        if let Some(task) = self.tasks.get_mut(&id) {
            for event in task.mark_failed("all sources failed") {
                self.push_event(event);
            }
        }
        Err(RuntimeError::AllPeersFailed { last_error })
    }

    pub async fn resume_verify(&mut self, id: TorrentId) -> Result<ResumeSummary, RuntimeError> {
        let task = self
            .tasks
            .get_mut(&id)
            .ok_or(RuntimeError::InvalidConfig("unknown torrent"))?;
        task.resume_verify().await
    }

    fn add_plan(&mut self, plan: TorrentPlan) -> Result<(), RuntimeError> {
        if self.tasks.len() >= self.config.limits.max_active_torrents {
            return Err(RuntimeError::Backpressure {
                stage: "adding torrent",
            });
        }
        let id = plan.id;
        if self.tasks.contains_key(&id) {
            return Err(RuntimeError::InvalidConfig("torrent already exists"));
        }
        self.tasks.insert(id, TorrentTask::new(plan));
        self.push_event(RuntimeEvent::TorrentAdded { torrent: id });
        Ok(())
    }

    pub fn add_plan_intent(&mut self, plan: TorrentPlan) -> Result<(), RuntimeError> {
        if self.tasks.len() >= self.config.limits.max_active_torrents {
            return Err(RuntimeError::Backpressure {
                stage: "adding torrent",
            });
        }
        let id = plan.id;
        self.tasks.insert(id, TorrentTask::new(plan));
        self.push_event(RuntimeEvent::TorrentAdded { torrent: id });
        Ok(())
    }

    pub fn remove_torrent_intent(
        &mut self,
        id: TorrentId,
    ) -> Result<Box<TorrentPlan>, RuntimeError> {
        let task = self
            .tasks
            .remove(&id)
            .ok_or(RuntimeError::InvalidConfig("unknown torrent"))?;
        self.push_event(RuntimeEvent::TorrentRemoved { torrent: id });
        Ok(Box::new(task.into_plan()))
    }

    pub fn apply_settings_patch(&mut self, patch: &SettingsPatch) -> Result<(), RuntimeError> {
        if let Some(port) = patch.listen_port {
            self.config.listen_port = port;
        }
        if let Some(limits) = patch.limits {
            self.config.limits = limits;
        }
        Ok(())
    }

    pub fn rollback(&mut self, record: RollbackRecord) -> Result<(), RuntimeError> {
        match record {
            RollbackRecord::AddRollback { id } => {
                self.tasks.remove(&id);
                Ok(())
            }
            RollbackRecord::RemoveRollback { id, plan } => {
                self.tasks
                    .entry(id)
                    .or_insert_with(|| TorrentTask::new(*plan));
                Ok(())
            }
            RollbackRecord::SettingsRollback { previous } => {
                self.config = *previous;
                Ok(())
            }
        }
    }

    fn apply_torrent(
        &mut self,
        id: TorrentId,
        command: TorrentCommand,
    ) -> Result<(), RuntimeError> {
        let task = self
            .tasks
            .get_mut(&id)
            .ok_or(RuntimeError::InvalidConfig("unknown torrent"))?;
        let events = task.apply(command)?;
        for event in events {
            self.push_event(event);
        }
        if command == TorrentCommand::Cancel {
            self.push_event(RuntimeEvent::TaskCancelled { torrent: id });
        }
        Ok(())
    }

    fn push_event(&mut self, event: RuntimeEvent) {
        if self.events.len() == self.config.limits.max_event_queue {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}
