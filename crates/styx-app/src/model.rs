use serde::{Deserialize, Serialize};

use crate::format::InfoHashHex;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub torrents: Vec<TorrentRow>,
    pub peers: Vec<PeerRow>,
    pub speed: Vec<SpeedSample>,
    pub logs: Vec<LogLine>,
    pub totals: SessionTotals,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TorrentRow {
    pub info_hash: InfoHashHex,
    pub name: String,
    pub status: TorrentStatus,
    pub size_bytes: u64,
    pub progress: f32,
    pub uploaded_bytes: u64,
    pub share_ratio: f32,
    pub down_rate: u64,
    pub up_rate: u64,
    pub peers: u32,
    pub seeds: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TorrentStatus {
    Checking,
    Paused,
    Downloading,
    Seeding,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeerRow {
    pub torrent: InfoHashHex,
    pub address: String,
    pub flags: String,
    pub progress: f32,
    pub down_rate: u64,
    pub up_rate: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpeedSample {
    pub second: u64,
    pub down_rate: u64,
    pub up_rate: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LogLine {
    pub level: LogLevel,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionTotals {
    pub down_bytes: u64,
    pub up_bytes: u64,
    pub torrent_count: u32,
    pub peer_count: u32,
}
