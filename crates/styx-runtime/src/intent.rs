use crate::{RuntimeConfig, RuntimeEngine, RuntimeError, SettingsPatch, TorrentId, TorrentPlan};

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
                if let Some(limits) = &patch.limits {
                    limits.clone().validate()?;
                }
                Ok(())
            }
        }
    }
}
