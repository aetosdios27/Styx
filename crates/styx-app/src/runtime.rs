use std::{collections::BTreeMap, fs, path::PathBuf};

use styx_proto::{decode_torrent, FileMode, TorrentMetainfo};

use crate::{
    commands::{CommandResponse, ControlCommand},
    error::AppError,
    events::AppEvent,
    format::InfoHashHex,
    model::{AppSnapshot, LogLevel, LogLine, SessionTotals, TorrentRow, TorrentStatus},
};

pub trait TorrentRuntime {
    fn apply(&mut self, command: ControlCommand) -> Result<CommandResponse, AppError>;
    fn snapshot(&mut self) -> AppSnapshot;
    fn tick(&mut self) -> Vec<AppEvent>;
}

#[derive(Debug, Default)]
pub struct MemoryRuntime {
    torrents: BTreeMap<InfoHashHex, TorrentEntry>,
    logs: Vec<LogLine>,
}

#[derive(Clone, Debug)]
struct TorrentEntry {
    row: TorrentRow,
}

impl TorrentRuntime for MemoryRuntime {
    fn apply(&mut self, command: ControlCommand) -> Result<CommandResponse, AppError> {
        match command {
            ControlCommand::Add {
                source,
                destination,
            } => self.add_torrent(source, destination),
            ControlCommand::Remove { info_hash } => {
                self.torrents
                    .remove(&info_hash)
                    .ok_or_else(|| AppError::UnknownTorrent(info_hash.to_string()))?;
                self.logs.push(LogLine {
                    level: LogLevel::Info,
                    message: format!("removed torrent {info_hash}"),
                });
                Ok(CommandResponse::TorrentRemoved { info_hash })
            }
            ControlCommand::Pause { info_hash } => {
                let entry = self.entry_mut(info_hash)?;
                entry.row.status = TorrentStatus::Paused;
                Ok(CommandResponse::TorrentPaused { info_hash })
            }
            ControlCommand::Resume { info_hash } => {
                let entry = self.entry_mut(info_hash)?;
                entry.row.status = TorrentStatus::Checking;
                Ok(CommandResponse::TorrentResumed { info_hash })
            }
            ControlCommand::Status => Ok(CommandResponse::Status {
                snapshot: self.snapshot(),
            }),
        }
    }

    fn snapshot(&mut self) -> AppSnapshot {
        let torrents = self
            .torrents
            .values()
            .map(|entry| entry.row.clone())
            .collect::<Vec<_>>();
        AppSnapshot {
            totals: SessionTotals {
                torrent_count: torrents.len() as u32,
                peer_count: 0,
                down_bytes: 0,
                up_bytes: 0,
            },
            torrents,
            peers: Vec::new(),
            speed: Vec::new(),
            logs: self.logs.clone(),
        }
    }

    fn tick(&mut self) -> Vec<AppEvent> {
        vec![AppEvent::Snapshot {
            snapshot: self.snapshot(),
        }]
    }
}

impl MemoryRuntime {
    fn add_torrent(
        &mut self,
        source: PathBuf,
        destination: Option<PathBuf>,
    ) -> Result<CommandResponse, AppError> {
        let bytes = fs::read(&source).map_err(|source_error| AppError::ReadTorrent {
            path: source.clone(),
            source: source_error,
        })?;
        let meta = decode_torrent(&bytes).map_err(|source_error| AppError::ParseTorrent {
            path: source.clone(),
            source: source_error,
        })?;
        let info_hash = InfoHashHex::new(*meta.info_hash_v1.as_bytes());
        if self.torrents.contains_key(&info_hash) {
            return Err(AppError::DuplicateTorrent(info_hash.to_string()));
        }

        let name = String::from_utf8_lossy(&meta.info.name).into_owned();
        let row = TorrentRow {
            info_hash,
            name: name.clone(),
            status: TorrentStatus::Checking,
            size_bytes: torrent_size(&meta),
            progress: 0.0,
            down_rate: 0,
            up_rate: 0,
            peers: 0,
            seeds: 0,
        };
        self.torrents.insert(info_hash, TorrentEntry { row });
        self.logs.push(LogLine {
            level: LogLevel::Info,
            message: destination.as_ref().map_or_else(
                || format!("added torrent {name}"),
                |path| format!("added torrent {name} to {}", path.display()),
            ),
        });
        Ok(CommandResponse::TorrentAdded { info_hash, name })
    }

    fn entry_mut(&mut self, info_hash: InfoHashHex) -> Result<&mut TorrentEntry, AppError> {
        self.torrents
            .get_mut(&info_hash)
            .ok_or_else(|| AppError::UnknownTorrent(info_hash.to_string()))
    }
}

fn torrent_size(meta: &TorrentMetainfo) -> u64 {
    match &meta.info.mode {
        FileMode::Single { length } => *length,
        FileMode::Multi { files } => files.iter().map(|file| file.length).sum(),
    }
}
