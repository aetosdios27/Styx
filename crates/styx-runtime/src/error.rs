use std::io;

use styx_disk::DiskError;
use styx_proto::{PeerWireError, TorrentMetainfoError};
use styx_tracker::TrackerError;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("invalid runtime config: {0}")]
    InvalidConfig(&'static str),
    #[error("torrent does not contain an HTTP tracker announce URL")]
    NoHttpTracker,
    #[error("invalid tracker URL `{url}`")]
    InvalidTrackerUrl { url: String },
    #[error("peer choked before serving the smoke piece")]
    PeerChoked,
    #[error("peer sent unexpected message while {stage}")]
    UnexpectedPeerMessage { stage: &'static str },
    #[error("piece {piece} failed hash verification")]
    PieceHashMismatch { piece: u32 },
    #[error("tracker announce returned no usable peers")]
    NoPeers,
    #[error("all peer smoke attempts failed: {last_error}")]
    AllPeersFailed { last_error: String },
    #[error("web seed smoke path supports single-file torrents only")]
    UnsupportedWebSeedLayout,
    #[error("web seed returned {actual} bytes for piece {piece}, expected {expected}")]
    InvalidWebSeedLength {
        piece: u32,
        expected: usize,
        actual: usize,
    },
    #[error("timed out while {stage}")]
    Timeout { stage: &'static str },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Torrent(#[from] TorrentMetainfoError),
    #[error(transparent)]
    Tracker(#[from] TrackerError),
    #[error(transparent)]
    PeerWire(#[from] PeerWireError),
    #[error(transparent)]
    Disk(#[from] DiskError),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

impl PartialEq for RuntimeError {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::InvalidConfig(left), Self::InvalidConfig(right)) if left == right
        ) || matches!((self, other), (Self::NoHttpTracker, Self::NoHttpTracker))
            || matches!((self, other), (Self::PeerChoked, Self::PeerChoked))
            || matches!(
                (self, other),
                (Self::InvalidTrackerUrl { url: left }, Self::InvalidTrackerUrl { url: right })
                    if left == right
            )
            || matches!(
                (self, other),
                (
                    Self::UnexpectedPeerMessage { stage: left },
                    Self::UnexpectedPeerMessage { stage: right }
                ) if left == right
            )
            || matches!(
                (self, other),
                (Self::PieceHashMismatch { piece: left }, Self::PieceHashMismatch { piece: right })
                    if left == right
            )
            || matches!((self, other), (Self::NoPeers, Self::NoPeers))
            || matches!(
                (self, other),
                (Self::AllPeersFailed { last_error: left }, Self::AllPeersFailed { last_error: right })
                    if left == right
            )
            || matches!(
                (self, other),
                (
                    Self::UnsupportedWebSeedLayout,
                    Self::UnsupportedWebSeedLayout
                )
            )
            || matches!(
                (self, other),
                (
                    Self::InvalidWebSeedLength {
                        piece: left_piece,
                        expected: left_expected,
                        actual: left_actual,
                    },
                    Self::InvalidWebSeedLength {
                        piece: right_piece,
                        expected: right_expected,
                        actual: right_actual,
                    },
                ) if left_piece == right_piece
                    && left_expected == right_expected
                    && left_actual == right_actual
            )
            || matches!(
                    (self, other),
                    (Self::Timeout { stage: left }, Self::Timeout { stage: right }) if left == right
            )
    }
}
