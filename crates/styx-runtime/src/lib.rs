//! Runtime orchestration for real-world Styx smoke tests.

mod error;
mod peer;
mod smoke;
mod torrent;
mod tracker;
mod types;

pub use error::RuntimeError;
pub use peer::{download_piece_from_peer, DownloadedPiece, PeerPieceRequest};
pub use smoke::{
    run_one_piece_smoke, run_one_piece_smoke_with_stream, run_one_piece_smoke_with_web_seed_bytes,
};
pub use torrent::{load_torrent_plan, TorrentSmokePlan};
pub use tracker::{build_started_announce, select_peer_candidates};
pub use types::{SmokeConfig, SmokeOutcome, SmokeRunConfig, SmokeStage, SmokeTarget};
