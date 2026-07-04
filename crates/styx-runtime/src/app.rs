use std::collections::VecDeque;

use styx_app::{
    commands::{CommandResponse, ControlCommand},
    error::AppError,
    events::AppEvent,
    format::InfoHashHex,
    model::{
        AppSnapshot, LogLevel, LogLine, SessionTotals, SpeedSample, TorrentRow,
        TorrentStatus as AppStatus,
    },
    TorrentRuntime,
};

use crate::{
    RuntimeCommand, RuntimeConfig, RuntimeEngine, RuntimeError, RuntimeEvent, RuntimeSnapshot,
    TorrentCommand, TorrentId, TorrentPlan, TorrentSnapshot, TorrentStatus,
};

const DEFAULT_SPEED_SAMPLES: usize = 60;
const MAX_LOG_LINES: usize = 1000;

#[derive(Debug)]
pub struct AppRuntime {
    engine: RuntimeEngine,
    speed: VecDeque<SpeedSample>,
    logs: VecDeque<LogLine>,
    tick_count: u64,
}

impl AppRuntime {
    pub fn new(engine: RuntimeEngine) -> Self {
        Self {
            engine,
            speed: VecDeque::with_capacity(DEFAULT_SPEED_SAMPLES),
            logs: VecDeque::with_capacity(MAX_LOG_LINES),
            tick_count: 0,
        }
    }

    pub fn into_engine(self) -> RuntimeEngine {
        self.engine
    }

    pub fn new_with_config(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        Ok(Self::new(RuntimeEngine::new(config)?))
    }

    fn apply_add(
        &mut self,
        source: std::path::PathBuf,
        destination: Option<std::path::PathBuf>,
    ) -> Result<CommandResponse, AppError> {
        let dest = destination
            .ok_or_else(|| AppError::InvalidCommand("destination required".into()))?;
        let plan = TorrentPlan::from_file(&source, &dest).map_err(|e| match e {
            RuntimeError::Io(io_err) => AppError::ReadTorrent {
                path: source.clone(),
                source: io_err,
            },
            RuntimeError::Torrent(parse_err) => AppError::ParseTorrent {
                path: source,
                source: parse_err,
            },
            other => AppError::Internal(other.to_string()),
        })?;
        let id = plan.id;
        let name = plan.name.clone();
        let info_hash = InfoHashHex::new(*id.as_bytes());
        self.engine
            .apply(RuntimeCommand::AddPlan(Box::new(plan)))
            .map_err(map_runtime_error)?;
        self.engine
            .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))
            .map_err(map_runtime_error)?;
        Ok(CommandResponse::TorrentAdded { info_hash, name })
    }
}

impl TorrentRuntime for AppRuntime {
    fn apply(&mut self, command: ControlCommand) -> Result<CommandResponse, AppError> {
        match command {
            ControlCommand::Add { source, destination } => self.apply_add(source, destination),
            ControlCommand::Remove { info_hash } => {
                let id = torrent_id_from_hex(info_hash)?;
                self.engine
                    .apply(RuntimeCommand::Remove(id))
                    .map_err(map_runtime_error)?;
                let info_hash_hex = InfoHashHex::new(*id.as_bytes());
                Ok(CommandResponse::TorrentRemoved {
                    info_hash: info_hash_hex,
                })
            }
            ControlCommand::Pause { info_hash } => {
                let id = torrent_id_from_hex(info_hash)?;
                self.engine
                    .apply(RuntimeCommand::Torrent(id, TorrentCommand::Pause))
                    .map_err(map_runtime_error)?;
                Ok(CommandResponse::TorrentPaused {
                    info_hash: InfoHashHex::new(*id.as_bytes()),
                })
            }
            ControlCommand::Resume { info_hash } => {
                let id = torrent_id_from_hex(info_hash)?;
                self.engine
                    .apply(RuntimeCommand::Torrent(id, TorrentCommand::Resume))
                    .map_err(map_runtime_error)?;
                Ok(CommandResponse::TorrentResumed {
                    info_hash: InfoHashHex::new(*id.as_bytes()),
                })
            }
            ControlCommand::Status => Ok(CommandResponse::Status {
                snapshot: self.snapshot(),
            }),
        }
    }

    fn snapshot(&self) -> AppSnapshot {
        let snap = self.engine.snapshot();
        let torrents: Vec<TorrentRow> = snap.torrents.iter().map(torrent_snapshot_to_row).collect();
        let totals = SessionTotals {
            torrent_count: torrents.len() as u32,
            peer_count: snap.peers.len() as u32,
            down_bytes: torrents.iter().map(|t| t.down_rate as u64).sum(),
            up_bytes: torrents.iter().map(|t| t.up_rate as u64).sum(),
        };
        AppSnapshot {
            torrents,
            peers: Vec::new(),
            speed: self.speed.iter().cloned().collect(),
            logs: self.logs.iter().cloned().collect(),
            totals,
        }
    }

    fn tick(&mut self) -> Vec<AppEvent> {
        let engine_events = self.engine.drain_events();
        let snap = self.engine.snapshot();

        let mut app_events = Vec::new();

        for event in engine_events {
            if let Some(app_event) = map_to_app_event(&event, &snap) {
                app_events.push(app_event);
            }
            if let Some(log) = map_to_log_line(&event) {
                if self.logs.len() >= MAX_LOG_LINES {
                    self.logs.pop_front();
                }
                self.logs.push_back(log);
            }
        }

        let total_down: u64 = snap.torrents.iter().map(|t| t.down_rate).sum();
        let total_up: u64 = snap.torrents.iter().map(|t| t.up_rate).sum();
        if self.speed.len() >= DEFAULT_SPEED_SAMPLES {
            self.speed.pop_front();
        }
        self.speed.push_back(SpeedSample {
            second: self.tick_count,
            down_rate: total_down,
            up_rate: total_up,
        });
        self.tick_count += 1;

        app_events
    }
}

fn torrent_id_from_hex(hex: InfoHashHex) -> Result<TorrentId, AppError> {
    use styx_proto::InfoHashV1;
    let bytes = *hex.as_bytes();
    Ok(TorrentId::new(InfoHashV1::new(bytes)))
}

fn map_runtime_error(err: RuntimeError) -> AppError {
    match err {
        RuntimeError::InvalidConfig(msg) => AppError::InvalidCommand(msg.to_string()),
        RuntimeError::Backpressure { .. } => AppError::Backpressure,
        _ => AppError::Internal(err.to_string()),
    }
}

fn map_to_app_event(event: &RuntimeEvent, snap: &RuntimeSnapshot) -> Option<AppEvent> {
    match event {
        RuntimeEvent::TorrentAdded { torrent } => {
            let info_hash = InfoHashHex::new(*torrent.as_bytes());
            let name = snap
                .torrents
                .iter()
                .find(|t| t.id == *torrent)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            Some(AppEvent::TorrentAdded { info_hash, name })
        }
        RuntimeEvent::TorrentRemoved { torrent } => Some(AppEvent::TorrentRemoved {
            info_hash: InfoHashHex::new(*torrent.as_bytes()),
        }),
        _ => None,
    }
}

fn map_to_log_line(event: &RuntimeEvent) -> Option<LogLine> {
    match event {
        RuntimeEvent::TorrentAdded { torrent } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("torrent {:?} added", torrent),
        }),
        RuntimeEvent::TorrentRemoved { torrent } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("torrent {:?} removed", torrent),
        }),
        RuntimeEvent::StateChanged { torrent, from, to } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("torrent {torrent:?}: {from:?} → {to:?}"),
        }),
        RuntimeEvent::SourceFailed {
            torrent, source, ..
        } => Some(LogLine {
            level: LogLevel::Warn,
            message: format!("source {source} failed for torrent {torrent:?}"),
        }),
        RuntimeEvent::TaskFailed { torrent, reason } => Some(LogLine {
            level: LogLevel::Error,
            message: format!("torrent {torrent:?} failed: {reason}"),
        }),
        RuntimeEvent::TaskCompleted { torrent } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("torrent {torrent:?} completed"),
        }),
        _ => None,
    }
}

fn torrent_snapshot_to_row(snap: &TorrentSnapshot) -> TorrentRow {
    let progress = snap.progress();
    TorrentRow {
        info_hash: InfoHashHex::new(*snap.id.as_bytes()),
        name: snap.name.clone(),
        status: match snap.status {
            TorrentStatus::Checking | TorrentStatus::Discovering => AppStatus::Checking,
            TorrentStatus::Downloading => AppStatus::Downloading,
            TorrentStatus::Paused => AppStatus::Paused,
            TorrentStatus::Complete | TorrentStatus::Seeding => AppStatus::Seeding,
            TorrentStatus::Failed | TorrentStatus::Cancelled => AppStatus::Error,
        },
        size_bytes: snap.total_bytes,
        progress,
        down_rate: snap.down_rate,
        up_rate: snap.up_rate,
        peers: snap.peers,
        seeds: snap.seeds,
    }
}
