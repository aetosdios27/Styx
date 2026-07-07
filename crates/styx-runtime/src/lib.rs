//! Runtime orchestration for real-world Styx smoke tests.

mod app;
mod config;
mod control;
mod download;
mod driver;
mod engine;
mod error;
mod intent;
mod peer;
mod peer_io;
mod peer_table;
mod persist;
mod rate;
mod session;
mod settings;
mod smoke;
mod snapshot;
mod source;
mod task;
mod torrent;
mod tracker;
mod types;
mod web_seed;

pub use app::AppRuntime;
pub use config::{RuntimeConfig, RuntimeLimits};
pub use control::{RuntimeCommand, TorrentCommand};
pub use download::run_full_v1_download;
pub use engine::RuntimeEngine;
pub use error::{FailureScope, RetryClass, RuntimeError};
pub use intent::{IntentState, RollbackRecord, StageIntent};
pub use peer::{download_piece_from_peer, DownloadedPiece, PeerPieceRequest};
pub use persist::{
    PersistentState, PersistentStore, PersistentTorrent, PersistentTorrentState,
    PERSISTENT_STATE_SCHEMA_VERSION,
};
pub use rate::RateCounter;
pub use session::{PeerSessionDriver, SessionFailure, SessionOutcome};
pub use settings::SettingsPatch;
pub use smoke::{
    run_one_piece_smoke, run_one_piece_smoke_with_stream, run_one_piece_smoke_with_web_seed_bytes,
};
pub use snapshot::{PeerSnapshot, RuntimeEvent, RuntimeSnapshot, TorrentSnapshot, TorrentStatus};
pub use source::{
    BlockCorruptionTracker, SourceCandidate, SourceEndpoint, SourceFailure, SourceId, SourceKind,
    SourceState, SourceTable,
};
pub use task::TorrentTask;
pub use torrent::{load_torrent_plan, TorrentId, TorrentPlan, TorrentSmokePlan};
pub use tracker::{build_started_announce, select_peer_candidates};
pub use types::{
    DownloadOutcome, DownloadRunConfig, SmokeConfig, SmokeOutcome, SmokeRunConfig, SmokeStage,
    SmokeTarget,
};
pub use web_seed::{piece_byte_range, validate_web_seed_piece_bytes, web_seed_file_url};
