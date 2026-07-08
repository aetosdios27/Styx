use std::time::Duration;

use bytes::Bytes;
use styx_disk::{BlockLength, BlockOffset, PieceIndex};
use styx_proto::PeerMessage;

use crate::CoreError;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerKey(u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TorrentKey([u8; 20]);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlockRequest {
    pub piece: PieceIndex,
    pub offset: BlockOffset,
    pub length: BlockLength,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisconnectReason {
    ProtocolViolation,
    DuplicatePeer,
    Stalled,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerAction {
    SendMessage {
        peer: PeerKey,
        message: PeerMessage,
    },
    Disconnect {
        peer: PeerKey,
        reason: DisconnectReason,
    },
    AcceptBlock {
        peer: PeerKey,
        request: BlockRequest,
        bytes: Bytes,
    },
    ServeBlock {
        peer: PeerKey,
        request: BlockRequest,
    },
    CancelDuplicate {
        peer: PeerKey,
        request: BlockRequest,
    },
    RecordInterest {
        peer: PeerKey,
        interested: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerManagerConfig {
    pub upload_slots: usize,
    pub request_pipeline_depth: usize,
    pub choke_interval: Duration,
    pub optimistic_unchoke_interval: Duration,
    pub rate_window: Duration,
    pub request_timeout: Duration,
    pub startup_random_pieces: usize,
}

impl PeerKey {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TorrentKey {
    #[must_use]
    pub const fn new(value: [u8; 20]) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }
}

impl BlockRequest {
    #[must_use]
    pub const fn new(piece: PieceIndex, offset: BlockOffset, length: BlockLength) -> Self {
        Self {
            piece,
            offset,
            length,
        }
    }
}

impl Default for PeerManagerConfig {
    fn default() -> Self {
        Self {
            upload_slots: 4,
            request_pipeline_depth: 5,
            choke_interval: Duration::from_secs(10),
            optimistic_unchoke_interval: Duration::from_secs(30),
            rate_window: Duration::from_secs(20),
            request_timeout: Duration::from_secs(30),
            startup_random_pieces: 4,
        }
    }
}

impl PeerManagerConfig {
    pub fn validate(self) -> Result<Self, CoreError> {
        if self.upload_slots == 0 {
            return Err(CoreError::InvalidConfig {
                field: "upload_slots",
            });
        }
        if self.request_pipeline_depth == 0 {
            return Err(CoreError::InvalidConfig {
                field: "request_pipeline_depth",
            });
        }
        if self.choke_interval.is_zero() {
            return Err(CoreError::InvalidConfig {
                field: "choke_interval",
            });
        }
        if self.optimistic_unchoke_interval.is_zero() {
            return Err(CoreError::InvalidConfig {
                field: "optimistic_unchoke_interval",
            });
        }
        if self.rate_window.is_zero() {
            return Err(CoreError::InvalidConfig {
                field: "rate_window",
            });
        }
        if self.request_timeout.is_zero() {
            return Err(CoreError::InvalidConfig {
                field: "request_timeout",
            });
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn default_config_uses_roadmap_transfer_policy_values() {
        let config = PeerManagerConfig::default();

        assert_eq!(config.upload_slots, 4);
        assert_eq!(config.request_pipeline_depth, 5);
        assert_eq!(config.choke_interval, Duration::from_secs(10));
        assert_eq!(config.optimistic_unchoke_interval, Duration::from_secs(30));
        assert_eq!(config.rate_window, Duration::from_secs(20));
        assert_eq!(config.request_timeout, Duration::from_secs(30));
    }

    #[test]
    fn validate_rejects_zero_upload_slots() {
        let config = PeerManagerConfig {
            upload_slots: 0,
            ..PeerManagerConfig::default()
        };

        let err = config.validate().unwrap_err();

        assert_eq!(
            err,
            CoreError::InvalidConfig {
                field: "upload_slots"
            }
        );
    }

    #[test]
    fn validate_rejects_zero_pipeline_depth() {
        let config = PeerManagerConfig {
            request_pipeline_depth: 0,
            ..PeerManagerConfig::default()
        };

        let err = config.validate().unwrap_err();

        assert_eq!(
            err,
            CoreError::InvalidConfig {
                field: "request_pipeline_depth"
            }
        );
    }

    #[test]
    fn validate_rejects_zero_rate_window() {
        let config = PeerManagerConfig {
            rate_window: Duration::ZERO,
            ..PeerManagerConfig::default()
        };

        let err = config.validate().unwrap_err();

        assert_eq!(
            err,
            CoreError::InvalidConfig {
                field: "rate_window"
            }
        );
    }
}
