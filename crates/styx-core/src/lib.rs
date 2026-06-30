//! Peer transfer policy for Styx.
//!
//! `styx-core` owns deterministic BitTorrent peer-management policy. It emits
//! effects for transport and disk drivers instead of performing socket or disk
//! side effects directly.

mod choke;
mod endgame;
mod error;
mod manager;
mod peer;
mod picker;
mod pipeline;
mod rate;
mod types;

pub use choke::{ChokeController, TransferMode};
pub use endgame::EndgameController;
pub use error::CoreError;
pub use manager::PeerConnectionManager;
pub use peer::PeerSession;
pub use picker::{PiecePicker, TorrentState};
pub use pipeline::{InFlightRequest, RequestPipeline};
pub use rate::RateWindow;
pub use types::{
    BlockRequest, DisconnectReason, PeerAction, PeerKey, PeerManagerConfig, TorrentKey,
};
