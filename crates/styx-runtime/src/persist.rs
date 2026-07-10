use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::RuntimeError;

pub const PERSISTENT_STATE_SCHEMA_VERSION: u16 = 2;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistentState {
    pub schema_version: u16,
    pub torrents: Vec<PersistentTorrent>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistentTorrent {
    pub source: PersistentTorrentSource,
    pub destination: PathBuf,
    pub state: PersistentTorrentState,
    pub added_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PersistentTorrentSource {
    File { path: PathBuf },
    Magnet { uri: String },
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

    pub(crate) fn validate(self) -> Result<Self, RuntimeError> {
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
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| RuntimeError::Persistence("invalid persistent state json"))?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or(RuntimeError::Persistence("invalid persistent state json"))?;
        let state = if version == 1 {
            let legacy: PersistentStateV1 = serde_json::from_value(value)
                .map_err(|_| RuntimeError::Persistence("invalid persistent state json"))?;
            legacy.into_current()
        } else {
            serde_json::from_value(value)
                .map_err(|_| RuntimeError::Persistence("invalid persistent state json"))?
        };
        state.validate()
    }

    pub fn save(&self, state: &PersistentState) -> Result<(), RuntimeError> {
        state.clone().validate()?;
        fs::create_dir_all(&self.state_dir)?;
        set_owner_only_directory(&self.state_dir)?;
        let tmp_path = self.state_dir.join("state.json.tmp");
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|_| RuntimeError::Persistence("failed to encode persistent state"))?;
        write_owner_only_file(&tmp_path, &bytes)?;
        fs::rename(tmp_path, &self.state_path)?;
        Ok(())
    }

    #[must_use]
    pub fn state_path(&self) -> &Path {
        &self.state_path
    }
}

#[cfg(unix)]
fn set_owner_only_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_owner_only_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn write_owner_only_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_owner_only_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    fs::write(path, bytes)
}

#[derive(Deserialize)]
struct PersistentStateV1 {
    torrents: Vec<PersistentTorrentV1>,
}

#[derive(Deserialize)]
struct PersistentTorrentV1 {
    source_path: PathBuf,
    destination: PathBuf,
    state: PersistentTorrentState,
    added_at_unix: u64,
    completed_at_unix: Option<u64>,
}

impl PersistentStateV1 {
    fn into_current(self) -> PersistentState {
        PersistentState {
            schema_version: PERSISTENT_STATE_SCHEMA_VERSION,
            torrents: self
                .torrents
                .into_iter()
                .map(|torrent| PersistentTorrent {
                    source: PersistentTorrentSource::File {
                        path: torrent.source_path,
                    },
                    destination: torrent.destination,
                    state: torrent.state,
                    added_at_unix: torrent.added_at_unix,
                    completed_at_unix: torrent.completed_at_unix,
                })
                .collect(),
        }
    }
}
