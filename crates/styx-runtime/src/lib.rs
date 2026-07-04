//! Runtime orchestration for real-world Styx smoke tests.

mod config;
mod control;
mod download;
mod engine;
mod error;
mod peer;
mod rate;
mod session;
mod smoke;
mod snapshot;
mod source;
mod task;
mod torrent;
mod tracker;
mod types;
mod web_seed;

pub use config::{RuntimeConfig, RuntimeLimits};
pub use control::{RuntimeCommand, TorrentCommand};
pub use download::run_full_v1_download;
pub use engine::RuntimeEngine;
pub use error::{FailureScope, RetryClass, RuntimeError};
pub use peer::{download_piece_from_peer, DownloadedPiece, PeerPieceRequest};
pub use rate::RateCounter;
pub use session::{PeerSessionDriver, SessionFailure, SessionOutcome};
pub use smoke::{
    run_one_piece_smoke, run_one_piece_smoke_with_stream, run_one_piece_smoke_with_web_seed_bytes,
};
pub use snapshot::{PeerSnapshot, RuntimeEvent, RuntimeSnapshot, TorrentSnapshot, TorrentStatus};
pub use source::{
    SourceCandidate, SourceEndpoint, SourceFailure, SourceId, SourceKind, SourceState, SourceTable,
};
pub use task::TorrentTask;
pub use torrent::{load_torrent_plan, TorrentId, TorrentPlan, TorrentSmokePlan};
pub use tracker::{build_started_announce, select_peer_candidates};
pub use types::{
    DownloadOutcome, DownloadRunConfig, SmokeConfig, SmokeOutcome, SmokeRunConfig, SmokeStage,
    SmokeTarget,
};
pub use web_seed::{piece_byte_range, validate_web_seed_piece_bytes, web_seed_file_url};
