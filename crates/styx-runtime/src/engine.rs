use std::collections::{BTreeMap, VecDeque};

use bytes::Bytes;
use styx_disk::{BlockSpec, PieceIndex};

use crate::{
    RuntimeCommand, RuntimeConfig, RuntimeError, RuntimeEvent, RuntimeSnapshot, TorrentCommand,
    TorrentId, TorrentPlan, TorrentTask,
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
