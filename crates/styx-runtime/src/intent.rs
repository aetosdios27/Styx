use crate::{RuntimeConfig, SettingsPatch, TorrentId, TorrentPlan};

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
}
