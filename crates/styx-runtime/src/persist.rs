use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::RuntimeError;

pub const PERSISTENT_STATE_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistentState {
    pub schema_version: u16,
    pub torrents: Vec<PersistentTorrent>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistentTorrent {
    pub source_path: PathBuf,
    pub destination: PathBuf,
    pub state: PersistentTorrentState,
    pub added_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistentTorrentState {
    Queued,
    Downloading,
    Paused,
    Complete,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistentStore {
    state_dir: PathBuf,
    state_path: PathBuf,
}

impl PersistentState {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: PERSISTENT_STATE_SCHEMA_VERSION,
            torrents: Vec::new(),
        }
    }

    fn validate(self) -> Result<Self, RuntimeError> {
        if self.schema_version != PERSISTENT_STATE_SCHEMA_VERSION {
            return Err(RuntimeError::Persistence(
                "unsupported persistent state schema version",
            ));
        }
        Ok(self)
    }
}

impl PersistentStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let state_dir = path.as_ref().to_path_buf();
        Ok(Self {
            state_path: state_dir.join("state.json"),
            state_dir,
        })
    }

    pub fn load(&self) -> Result<PersistentState, RuntimeError> {
        if !self.state_path.exists() {
            return Ok(PersistentState::empty());
        }
        let bytes = fs::read(&self.state_path)?;
        let state: PersistentState = serde_json::from_slice(&bytes)
            .map_err(|_| RuntimeError::Persistence("invalid persistent state json"))?;
        state.validate()
    }

    pub fn save(&self, state: &PersistentState) -> Result<(), RuntimeError> {
        state.clone().validate()?;
        fs::create_dir_all(&self.state_dir)?;
        let tmp_path = self.state_dir.join("state.json.tmp");
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|_| RuntimeError::Persistence("failed to encode persistent state"))?;
        fs::write(&tmp_path, bytes)?;
        fs::rename(tmp_path, &self.state_path)?;
        Ok(())
    }

    #[must_use]
    pub fn state_path(&self) -> &Path {
        &self.state_path
    }
}
