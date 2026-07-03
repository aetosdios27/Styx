//! Shared app-facing contracts for Styx user interfaces.

pub mod commands;
pub mod error;
pub mod events;
pub mod format;
pub mod model;
pub mod runtime;

pub use commands::{CommandEnvelope, CommandResponse, CommandResponseEnvelope, ControlCommand};
pub use error::AppError;
pub use events::AppEvent;
pub use format::{format_bytes, format_percent, format_rate, sparkline, InfoHashHex};
pub use model::{
    AppSnapshot, LogLevel, LogLine, PeerRow, SessionTotals, SpeedSample, TorrentRow, TorrentStatus,
};
pub use runtime::{MemoryRuntime, TorrentRuntime};
