use serde::{Deserialize, Serialize};

use crate::{format::InfoHashHex, model::AppSnapshot};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEvent {
    DaemonStarted {
        ipc: Option<String>,
        at_ms: u64,
    },
    TorrentAdded {
        info_hash: InfoHashHex,
        name: String,
    },
    TorrentRemoved {
        info_hash: InfoHashHex,
    },
    Snapshot {
        snapshot: AppSnapshot,
    },
    CommandFailed {
        command: String,
        error: String,
    },
}
