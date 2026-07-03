use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("info hash must be 40 hex characters")]
    InvalidInfoHashLength,
    #[error("info hash contains non-hex character `{byte}` at index {index}")]
    InvalidInfoHashHex { index: usize, byte: char },
    #[error("unknown torrent `{0}`")]
    UnknownTorrent(String),
    #[error("torrent `{0}` already exists")]
    DuplicateTorrent(String),
    #[error("failed to read torrent `{path}`: {source}")]
    ReadTorrent {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse torrent `{path}`: {source}")]
    ParseTorrent {
        path: PathBuf,
        #[source]
        source: styx_proto::TorrentMetainfoError,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
