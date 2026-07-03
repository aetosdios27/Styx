use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{format::InfoHashHex, model::AppSnapshot};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlCommand {
    Add {
        source: PathBuf,
        destination: Option<PathBuf>,
    },
    Remove {
        info_hash: InfoHashHex,
    },
    Pause {
        info_hash: InfoHashHex,
    },
    Resume {
        info_hash: InfoHashHex,
    },
    Status,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandResponse {
    TorrentAdded {
        info_hash: InfoHashHex,
        name: String,
    },
    TorrentRemoved {
        info_hash: InfoHashHex,
    },
    TorrentPaused {
        info_hash: InfoHashHex,
    },
    TorrentResumed {
        info_hash: InfoHashHex,
    },
    Status {
        snapshot: AppSnapshot,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub version: u16,
    pub command: ControlCommand,
}

impl CommandEnvelope {
    #[must_use]
    pub const fn new(command: ControlCommand) -> Self {
        Self {
            version: 1,
            command,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandResponseEnvelope {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<CommandResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CommandResponseEnvelope {
    #[must_use]
    pub fn ok(response: CommandResponse) -> Self {
        Self {
            ok: true,
            response: Some(response),
            error: None,
        }
    }

    #[must_use]
    pub fn err(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            response: None,
            error: Some(error.into()),
        }
    }
}
