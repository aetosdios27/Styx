use crate::{RuntimeConfig, RuntimeEngine, RuntimeError, RuntimeEvent, SettingsPatch, TorrentCommand, TorrentId, TorrentPlan};

#[derive(Clone, Debug)]
pub enum StageIntent {
    Add { plan: Box<TorrentPlan> },
    Remove { id: TorrentId, delete_data: bool },
    Pause { id: TorrentId },
    Resume { id: TorrentId },
    Settings { patch: SettingsPatch },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntentState {
    Declared,
    Validated,
    Executed,
    Failed,
    RolledBack,
}

#[derive(Clone, Debug)]
pub enum RollbackRecord {
    AddRollback {
        id: TorrentId,
    },
    RemoveRollback {
        id: TorrentId,
        plan: Box<TorrentPlan>,
    },
    SettingsRollback {
        previous: Box<RuntimeConfig>,
    },
}

impl StageIntent {
    pub fn state(&self) -> IntentState {
        IntentState::Declared
    }

    pub fn execute(
        &self,
        engine: &mut RuntimeEngine,
    ) -> Result<Option<RollbackRecord>, RuntimeError> {
        match self {
            Self::Add { plan } => {
                let id = plan.id;
                engine.add_plan_intent((**plan).clone())?;
                Ok(Some(RollbackRecord::AddRollback { id }))
            }
            Self::Remove { id, delete_data: _ } => {
                let plan = engine.remove_torrent_intent(*id)?;
                Ok(Some(RollbackRecord::RemoveRollback { id: *id, plan }))
            }
            Self::Pause { id } => {
                engine.apply_torrent(*id, TorrentCommand::Pause)?;
                Ok(None)
            }
            Self::Resume { id } => {
                engine.apply_torrent(*id, TorrentCommand::Resume)?;
                Ok(None)
            }
            Self::Settings { patch } => {
                let previous = engine.config().clone();
                engine.apply_settings_patch(patch)?;
                Ok(Some(RollbackRecord::SettingsRollback {
                    previous: Box::new(previous),
                }))
            }
        }
    }

    pub fn declare(&self) -> Vec<RuntimeEvent> {
        vec![RuntimeEvent::IntentDeclared {
            torrent: self.torrent_id(),
            kind: self.kind_str(),
        }]
    }

    pub fn run(&self, engine: &mut RuntimeEngine) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let mut events = self.declare();

        events.push(RuntimeEvent::ValidationStarted);
        self.validate(engine)?;
        events.push(RuntimeEvent::ValidationSucceeded);

        events.push(RuntimeEvent::ExecutionStarted);
        self.execute(engine)?;
        events.push(RuntimeEvent::ExecutionSucceeded);

        Ok(events)
    }

    fn torrent_id(&self) -> Option<TorrentId> {
        match self {
            Self::Add { plan } => Some(plan.id),
            Self::Remove { id, .. } => Some(*id),
            Self::Pause { id } => Some(*id),
            Self::Resume { id } => Some(*id),
            Self::Settings { .. } => None,
        }
    }

    fn kind_str(&self) -> &'static str {
        match self {
            Self::Add { .. } => "add",
            Self::Remove { .. } => "remove",
            Self::Pause { .. } => "pause",
            Self::Resume { .. } => "resume",
            Self::Settings { .. } => "settings",
        }
    }

    pub fn validate(&self, engine: &RuntimeEngine) -> Result<(), RuntimeError> {
        match self {
            Self::Add { plan } => {
                if engine.has_torrent(plan.id) {
                    return Err(RuntimeError::InvalidConfig("torrent already exists"));
                }
                Ok(())
            }
            Self::Remove { id, .. } | Self::Pause { id } | Self::Resume { id } => {
                if !engine.has_torrent(*id) {
                    return Err(RuntimeError::InvalidConfig("unknown torrent"));
                }
                Ok(())
            }
            Self::Settings { patch } => {
                if let Some(port) = patch.listen_port {
                    if port == 0 {
                        return Err(RuntimeError::InvalidConfig("listen port must be non-zero"));
                    }
                }
                if let Some(limits) = patch.limits {
                    limits.validate()?;
                }
                Ok(())
            }
        }
    }
}
