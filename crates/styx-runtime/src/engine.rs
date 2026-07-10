use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;

use bytes::Bytes;
use styx_core::{PeerIdentityManager, PrivacyConfig};
use styx_disk::{BlockSpec, PieceIndex, ResumeSummary};

use crate::{
    BlockCorruptionTracker, RollbackRecord, RuntimeCommand, RuntimeConfig, RuntimeError,
    RuntimeEvent, RuntimeSnapshot, SettingsPatch, StageIntent, TorrentCommand, TorrentId,
    TorrentPlan, TorrentTask,
};

#[derive(Debug)]
pub struct RuntimeEngine {
    config: RuntimeConfig,
    tasks: BTreeMap<TorrentId, TorrentTask>,
    events: VecDeque<RuntimeEvent>,
    identities: PeerIdentityManager,
    pub block_corruption: BlockCorruptionTracker,
}

impl RuntimeEngine {
    pub fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        Ok(Self {
            config: config.validate()?,
            tasks: BTreeMap::new(),
            events: VecDeque::new(),
            identities: PeerIdentityManager::new(PrivacyConfig::default()).map_err(|_| {
                RuntimeError::InvalidConfig("default privacy configuration must be valid")
            })?,
            block_corruption: BlockCorruptionTracker::new(3),
        })
    }

    pub fn has_torrent(&self, id: TorrentId) -> bool {
        self.tasks.contains_key(&id)
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn apply(&mut self, command: RuntimeCommand) -> Result<(), RuntimeError> {
        let intent = match command {
            RuntimeCommand::AddPlan(plan) => StageIntent::Add { plan },
            RuntimeCommand::AddMagnet(magnet) => StageIntent::AddMagnet { magnet },
            RuntimeCommand::Remove(id) => StageIntent::Remove {
                id,
                delete_data: false,
            },
            RuntimeCommand::Torrent(id, cmd) => match cmd {
                TorrentCommand::Pause => StageIntent::Pause { id },
                TorrentCommand::Resume => StageIntent::Resume { id },
                other => {
                    let events = self.apply_torrent(id, other)?;
                    for event in events {
                        self.push_event(event);
                    }
                    return Ok(());
                }
            },
        };
        let (events, result) = intent.run(self);
        for event in events {
            self.push_event(event);
        }
        result
    }

    #[must_use]
    pub fn snapshot(&mut self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            torrents: self.tasks.values_mut().map(TorrentTask::snapshot).collect(),
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
            for event in task.mark_failed(last_error.as_str()) {
                self.push_event(event);
            }
        }
        Err(RuntimeError::AllPeersFailed { last_error })
    }

    pub fn add_dht_peers(
        &mut self,
        id: TorrentId,
        peers: Vec<SocketAddr>,
    ) -> Result<usize, RuntimeError> {
        let task = self
            .tasks
            .get_mut(&id)
            .ok_or(RuntimeError::InvalidConfig("unknown torrent"))?;
        let added = task.add_dht_peers(peers);
        if added > 0 {
            self.push_event(RuntimeEvent::DhtPeersDiscovered {
                torrent: id,
                peers: u32::try_from(added).unwrap_or(u32::MAX),
            });
        }
        Ok(added)
    }

    #[must_use]
    pub fn dht_announce_targets(&self) -> Vec<(TorrentId, styx_dht::InfoHash)> {
        self.tasks
            .iter()
            .filter_map(|(id, task)| task.dht_announce_target().map(|hash| (*id, hash)))
            .collect()
    }

    #[must_use]
    pub fn dht_announce_target(&self, id: TorrentId) -> Option<styx_dht::InfoHash> {
        self.tasks.get(&id)?.dht_announce_target()
    }

    #[must_use]
    pub fn lsd_announce_targets(&self) -> Vec<(TorrentId, styx_proto::InfoHashV1)> {
        self.tasks
            .iter()
            .filter_map(|(id, task)| task.lsd_announce_target().map(|hash| (*id, hash)))
            .collect()
    }

    pub fn add_lsd_peer(&mut self, id: TorrentId, peer: SocketAddr) -> bool {
        self.tasks
            .get_mut(&id)
            .is_some_and(|task| task.add_lsd_peer(peer))
    }

    pub async fn resume_verify(&mut self, id: TorrentId) -> Result<ResumeSummary, RuntimeError> {
        let task = self
            .tasks
            .get_mut(&id)
            .ok_or(RuntimeError::InvalidConfig("unknown torrent"))?;
        task.resume_verify().await
    }

    pub fn add_plan_intent(
        &mut self,
        plan: TorrentPlan,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        if self.tasks.len() >= self.config.limits.max_active_torrents {
            return Err(RuntimeError::Backpressure {
                stage: "adding torrent",
            });
        }
        let id = plan.id;
        let task = self.task_with_fresh_identity(plan)?;
        self.tasks.insert(id, task);
        Ok(vec![RuntimeEvent::TorrentAdded { torrent: id }])
    }

    pub fn remove_torrent_intent(
        &mut self,
        id: TorrentId,
    ) -> Result<(Box<TorrentPlan>, Vec<RuntimeEvent>), RuntimeError> {
        let task = self
            .tasks
            .remove(&id)
            .ok_or(RuntimeError::InvalidConfig("unknown torrent"))?;
        let plan = task.into_plan();
        Ok((
            Box::new(plan),
            vec![RuntimeEvent::TorrentRemoved { torrent: id }],
        ))
    }

    pub fn apply_settings_patch(&mut self, patch: &SettingsPatch) -> Result<(), RuntimeError> {
        if let Some(port) = patch.listen_port {
            self.config.listen_port = port;
        }
        if let Some(limits) = patch.limits {
            self.config.limits = limits;
        }
        if let Some(seed_policy) = patch.seed_policy {
            self.config.seed_policy = seed_policy;
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
                if !self.tasks.contains_key(&id) {
                    let task = self.task_with_fresh_identity(*plan)?;
                    self.tasks.insert(id, task);
                }
                Ok(())
            }
            RollbackRecord::SettingsRollback { previous } => {
                self.config = *previous;
                Ok(())
            }
        }
    }

    pub(crate) fn apply_torrent(
        &mut self,
        id: TorrentId,
        command: TorrentCommand,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let task = self
            .tasks
            .get_mut(&id)
            .ok_or(RuntimeError::InvalidConfig("unknown torrent"))?;
        let mut events = task.apply(command)?;
        if command == TorrentCommand::Cancel {
            events.push(RuntimeEvent::TaskCancelled { torrent: id });
        }
        Ok(events)
    }

    pub fn sync_progress(
        &mut self,
        id: TorrentId,
        verified_bytes: u64,
    ) -> Result<(), RuntimeError> {
        let task = self
            .tasks
            .get_mut(&id)
            .ok_or(RuntimeError::InvalidConfig("unknown torrent"))?;
        task.set_verified_bytes(verified_bytes);
        Ok(())
    }

    pub fn replace_with_completed(
        &mut self,
        id: TorrentId,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let (plan, mut events) = self.remove_torrent_intent(id)?;
        let total_size = plan.total_size;
        let mut task = self.task_with_fresh_identity(*plan)?;
        task.set_status_complete()?;
        if self.config.seed_policy.seed_after_complete {
            events.extend(task.start_seeding()?);
        }
        self.tasks.insert(id, task);
        events.push(RuntimeEvent::TorrentAdded { torrent: id });
        events.push(RuntimeEvent::ProgressUpdated {
            torrent: id,
            verified_bytes: total_size,
            total_bytes: total_size,
        });
        events.push(RuntimeEvent::TaskCompleted { torrent: id });
        Ok(events)
    }

    pub fn record_block_failure(&mut self, piece: u32, block: u32, peer: SocketAddr) -> bool {
        self.block_corruption.record_failure(piece, block, peer)
    }

    fn task_with_fresh_identity(&mut self, plan: TorrentPlan) -> Result<TorrentTask, RuntimeError> {
        let identity = self
            .identities
            .generate(&mut rand::rng())
            .map_err(|_| RuntimeError::InvalidConfig("peer identity generation exhausted"))?;
        TorrentTask::new_with_peers_and_peer_id(plan, self.config.clone(), identity.peer_id)
    }

    pub fn push_event(&mut self, event: RuntimeEvent) {
        if self.events.len() == self.config.limits.max_event_queue {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}
