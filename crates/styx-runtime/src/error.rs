use std::io;

use styx_disk::DiskError;
use styx_proto::{PeerWireError, TorrentMetainfoError};
use styx_tracker::TrackerError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureScope {
    SourceLocal,
    TorrentGlobal,
    RuntimeGlobal,
    IntentStage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryClass {
    Retryable,
    Quarantine,
    Terminal,
    Rollbackable,
}

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
    #[error("{scope:?} source `{source_id}` failed ({retry:?}): {reason}")]
    SourceFailed {
        source_id: String,
        scope: FailureScope,
        retry: RetryClass,
        reason: String,
    },
    #[error("runtime backpressure while {stage}")]
    Backpressure { stage: &'static str },
    #[error("runtime task was cancelled")]
    Cancelled,
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

impl RuntimeError {
    #[must_use]
    pub const fn scope(&self) -> FailureScope {
        match self {
            Self::SourceFailed { scope, .. } => *scope,
            Self::Backpressure { .. } | Self::Cancelled | Self::InvalidConfig(_) => {
                FailureScope::RuntimeGlobal
            }
            Self::NoHttpTracker
            | Self::NoPeers
            | Self::AllPeersFailed { .. }
            | Self::UnsupportedWebSeedLayout
            | Self::PieceHashMismatch { .. }
            | Self::InvalidWebSeedLength { .. } => FailureScope::TorrentGlobal,
            Self::InvalidTrackerUrl { .. }
            | Self::PeerChoked
            | Self::UnexpectedPeerMessage { .. }
            | Self::Timeout { .. }
            | Self::Io(_)
            | Self::Torrent(_)
            | Self::Tracker(_)
            | Self::PeerWire(_)
            | Self::Disk(_)
            | Self::Http(_) => FailureScope::SourceLocal,
        }
    }

    #[must_use]
    pub const fn retry_class(&self) -> RetryClass {
        match self {
            Self::SourceFailed { retry, .. } => *retry,
            Self::Timeout { .. } | Self::PeerChoked | Self::NoPeers => RetryClass::Retryable,
            Self::PieceHashMismatch { .. }
            | Self::UnexpectedPeerMessage { .. }
            | Self::InvalidWebSeedLength { .. } => RetryClass::Quarantine,
            Self::Backpressure { .. } => RetryClass::Retryable,
            Self::AllPeersFailed { .. }
            | Self::NoHttpTracker
            | Self::UnsupportedWebSeedLayout
            | Self::InvalidConfig(_)
            | Self::InvalidTrackerUrl { .. }
            | Self::Cancelled
            | Self::Io(_)
            | Self::Torrent(_)
            | Self::Tracker(_)
            | Self::PeerWire(_)
            | Self::Disk(_)
            | Self::Http(_) => RetryClass::Terminal,
        }
    }
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
            || matches!(
                (self, other),
                (
                    Self::SourceFailed {
                        source_id: left_source,
                        scope: left_scope,
                        retry: left_retry,
                        reason: left_reason,
                    },
                    Self::SourceFailed {
                        source_id: right_source,
                        scope: right_scope,
                        retry: right_retry,
                        reason: right_reason,
                    },
                ) if left_source == right_source
                    && left_scope == right_scope
                    && left_retry == right_retry
                    && left_reason == right_reason
            )
            || matches!(
                (self, other),
                (Self::Backpressure { stage: left }, Self::Backpressure { stage: right }) if left == right
            )
            || matches!((self, other), (Self::Cancelled, Self::Cancelled))
    }
}
