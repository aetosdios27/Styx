//! Runtime orchestration for real-world Styx smoke tests.

mod app;
mod config;
mod control;
mod daemon;
mod dht;
mod discovery;
mod download;
mod driver;
mod engine;
mod error;
mod intent;
mod lsd;
mod magnet;
mod metadata;
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
mod supervision;
mod task;
mod torrent;
mod tracker;
mod types;
mod web_seed;

pub use app::{AppRuntime, PersistentAppRuntime};
pub use config::{RuntimeConfig, RuntimeLimits, SeedPolicy, SessionRuntimeConfig};
pub use control::{RuntimeCommand, TorrentCommand};
pub use daemon::{DaemonConfig, DaemonHandle, DaemonRuntime, DaemonStatus};
pub use dht::{
    spawn_dht_worker, DhtClient, DhtCommand, DhtOwner, DhtRuntimeConfig, DhtRuntimeEvent,
};
pub use discovery::DiscoveryPolicy;
pub use download::run_full_v1_download;
pub use engine::RuntimeEngine;
pub use error::{FailureScope, RetryClass, RuntimeError};
pub use intent::{IntentState, RollbackRecord, StageIntent};
pub use lsd::{
    decode_lsd_announce, encode_lsd_announce, spawn_lsd_worker, LsdAnnounce, LsdClient, LsdCommand,
    LsdDiscovery, LsdError, LsdOwner, LsdRuntimeEvent, LSD_IPV4_MULTICAST, LSD_IPV6_MULTICAST,
    MAX_LSD_PACKET_BYTES,
};
pub use magnet::{resolve_magnet_from_exact_peers, MagnetAdd, ResolvedMagnet};
pub use metadata::{fetch_metadata_from_peer, fetch_metadata_from_stream, MetadataFetchConfig};
pub use peer::{download_piece_from_peer, DownloadedPiece, PeerPieceRequest};
pub use persist::{
    PersistentState, PersistentStore, PersistentTorrent, PersistentTorrentSource,
    PersistentTorrentState, PERSISTENT_STATE_SCHEMA_VERSION,
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
pub use supervision::{
    spawn_session_supervisor, FailureReasonCode, OwnedTask, PersistenceOutcome, SessionClient,
    SessionEventStream, SessionNotice, SessionOwner, SharedWorkerKind, ShutdownMode,
    ShutdownReport, TaskExit, TaskKind, TaskRegistry,
};
pub use task::TorrentTask;
pub use torrent::{load_torrent_plan, TorrentId, TorrentPlan, TorrentSmokePlan};
pub use tracker::{build_started_announce, select_peer_candidates};
pub use types::{
    DownloadOutcome, DownloadRunConfig, SmokeConfig, SmokeOutcome, SmokeRunConfig, SmokeStage,
    SmokeTarget,
};
pub use web_seed::{piece_byte_range, validate_web_seed_piece_bytes, web_seed_file_url};
