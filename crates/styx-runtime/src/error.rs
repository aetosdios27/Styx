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
    #[error("v2-only torrents require hybrid support or a v2-capable transport")]
    V2NotSupported,
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
    #[error("torrent does not contain any web seed URLs")]
    NoWebSeeds,
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
    #[error("v2 integrity check failed: {0}")]
    V2IntegrityCheckFailed(&'static str),
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
            | Self::NoWebSeeds
            | Self::V2NotSupported
            | Self::V2IntegrityCheckFailed(_)
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
            Self::Timeout { .. } | Self::NoPeers => RetryClass::Retryable,
            Self::PeerChoked => RetryClass::Terminal,
            Self::PieceHashMismatch { .. }
            | Self::UnexpectedPeerMessage { .. }
            | Self::InvalidWebSeedLength { .. } => RetryClass::Quarantine,
            Self::Backpressure { .. } => RetryClass::Retryable,
            Self::AllPeersFailed { .. }
            | Self::NoHttpTracker
            | Self::NoWebSeeds
            | Self::V2NotSupported
            | Self::V2IntegrityCheckFailed(_)
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
            || matches!((self, other), (Self::V2NotSupported, Self::V2NotSupported))
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
            || matches!((self, other), (Self::NoWebSeeds, Self::NoWebSeeds))
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
                (Self::V2IntegrityCheckFailed(left), Self::V2IntegrityCheckFailed(right)) if left == right
            )
            || matches!(
                (self, other),
                (Self::Backpressure { stage: left }, Self::Backpressure { stage: right }) if left == right
            )
            || matches!((self, other), (Self::Cancelled, Self::Cancelled))
            || matches!((self, other), (Self::Io(_), Self::Io(_)))
            || matches!(
                (self, other),
                (Self::Torrent(a), Self::Torrent(b)) if a == b
            )
            || matches!((self, other), (Self::Tracker(_), Self::Tracker(_)))
            || matches!((self, other), (Self::PeerWire(_), Self::PeerWire(_)))
            || matches!(
                (self, other),
                (Self::Disk(a), Self::Disk(b)) if a == b
            )
            || matches!((self, other), (Self::Http(_), Self::Http(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_eq_torrent_variant_compares_inner_value() {
        use styx_proto::TorrentMetainfoError;

        let a = RuntimeError::Torrent(TorrentMetainfoError::MissingInfo);
        let b = RuntimeError::Torrent(TorrentMetainfoError::MissingInfo);
        let c = RuntimeError::Torrent(TorrentMetainfoError::MissingField {
            field: "name",
            context: "info",
        });
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn partial_eq_disk_variant_compares_inner_value() {
        use styx_disk::DiskError;

        let a = RuntimeError::Disk(DiskError::HashMismatch);
        let b = RuntimeError::Disk(DiskError::HashMismatch);
        let c = RuntimeError::Disk(DiskError::InvalidPieceLength);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn partial_eq_io_variant_uses_discriminant() {
        let a = RuntimeError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, ""));
        let b = RuntimeError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, ""));
        let c = RuntimeError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "",
        ));
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_ne!(a, RuntimeError::Cancelled);
    }

    #[test]
    fn partial_eq_tracker_uses_discriminant() {
        let a = RuntimeError::Tracker(TrackerError::InvalidUrl);
        let b = RuntimeError::Tracker(TrackerError::InvalidUrl);
        assert_eq!(a, b);
        assert_ne!(a, RuntimeError::Cancelled);
    }

    #[test]
    fn partial_eq_peer_wire_uses_discriminant() {
        let a = RuntimeError::PeerWire(PeerWireError::InfoHashMismatch);
        let b = RuntimeError::PeerWire(PeerWireError::InfoHashMismatch);
        assert_eq!(a, b);
        assert_ne!(a, RuntimeError::Cancelled);
    }

    #[test]
    fn partial_eq_existing_unit_variants_still_work() {
        assert_eq!(RuntimeError::Cancelled, RuntimeError::Cancelled);
        assert_eq!(RuntimeError::NoHttpTracker, RuntimeError::NoHttpTracker);
        assert_eq!(RuntimeError::PeerChoked, RuntimeError::PeerChoked);
        assert_eq!(RuntimeError::NoWebSeeds, RuntimeError::NoWebSeeds);
        assert_ne!(RuntimeError::Cancelled, RuntimeError::NoHttpTracker);
        assert_ne!(RuntimeError::NoWebSeeds, RuntimeError::NoPeers);
    }

    #[test]
    fn no_web_seeds_error_string() {
        assert_eq!(
            RuntimeError::NoWebSeeds.to_string(),
            "torrent does not contain any web seed URLs"
        );
    }

    #[test]
    fn no_web_seeds_scope_and_retry() {
        assert_eq!(
            RuntimeError::NoWebSeeds.scope(),
            FailureScope::TorrentGlobal
        );
        assert_eq!(RuntimeError::NoWebSeeds.retry_class(), RetryClass::Terminal);
    }
}
