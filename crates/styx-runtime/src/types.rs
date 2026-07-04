use std::{path::PathBuf, time::Duration};

use styx_proto::PeerId;

use crate::RuntimeError;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SmokeTarget {
    #[default]
    FirstPiece,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmokeStage {
    LoadingTorrent,
    Announcing,
    ConnectingPeer,
    Handshaking,
    DownloadingPiece,
    Verifying,
    Verified,
}

impl SmokeStage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LoadingTorrent => "loading_torrent",
            Self::Announcing => "announcing",
            Self::ConnectingPeer => "connecting_peer",
            Self::Handshaking => "handshaking",
            Self::DownloadingPiece => "downloading_piece",
            Self::Verifying => "verifying",
            Self::Verified => "verified",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmokeConfig {
    pub connect_timeout: Duration,
    pub peer_message_timeout: Duration,
    pub max_tracker_response_bytes: usize,
    pub numwant: u32,
    pub target: SmokeTarget,
}

impl Default for SmokeConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            peer_message_timeout: Duration::from_secs(15),
            max_tracker_response_bytes: 512 * 1024,
            numwant: 30,
            target: SmokeTarget::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmokeRunConfig {
    pub torrent_path: PathBuf,
    pub destination: PathBuf,
    pub listen_port: u16,
    pub peer_id: PeerId,
    pub limits: SmokeConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadRunConfig {
    pub torrent_path: PathBuf,
    pub destination: PathBuf,
    pub listen_port: u16,
    pub limits: SmokeConfig,
}

impl DownloadRunConfig {
    #[must_use]
    pub fn default_for_paths(
        torrent_path: impl Into<PathBuf>,
        destination: impl Into<PathBuf>,
    ) -> Self {
        Self {
            torrent_path: torrent_path.into(),
            destination: destination.into(),
            listen_port: 6881,
            limits: SmokeConfig::default(),
        }
    }

    pub fn validate(&self) -> Result<(), RuntimeError> {
        self.limits.validate()?;
        if self.listen_port == 0 {
            return Err(RuntimeError::InvalidConfig(
                "listen_port must be greater than zero",
            ));
        }
        Ok(())
    }
}

impl SmokeRunConfig {
    #[must_use]
    pub fn default_for_paths(
        torrent_path: impl Into<PathBuf>,
        destination: impl Into<PathBuf>,
    ) -> Self {
        Self {
            torrent_path: torrent_path.into(),
            destination: destination.into(),
            listen_port: 6881,
            peer_id: PeerId::new(random_peer_id_bytes()),
            limits: SmokeConfig::default(),
        }
    }

    pub fn validate(&self) -> Result<(), RuntimeError> {
        self.limits.validate()?;
        if self.listen_port == 0 {
            return Err(RuntimeError::InvalidConfig(
                "listen_port must be greater than zero",
            ));
        }
        Ok(())
    }
}

impl SmokeConfig {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.connect_timeout.is_zero() {
            return Err(RuntimeError::InvalidConfig(
                "connect_timeout must be greater than zero",
            ));
        }
        if self.peer_message_timeout.is_zero() {
            return Err(RuntimeError::InvalidConfig(
                "peer_message_timeout must be greater than zero",
            ));
        }
        if self.max_tracker_response_bytes == 0 {
            return Err(RuntimeError::InvalidConfig(
                "max_tracker_response_bytes must be greater than zero",
            ));
        }
        if self.numwant == 0 {
            return Err(RuntimeError::InvalidConfig(
                "numwant must be greater than zero",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SmokeOutcome {
    Verified { piece: u32, bytes: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownloadOutcome {
    Complete { pieces: u32, bytes: u64 },
}

impl DownloadOutcome {
    #[must_use]
    pub const fn pieces(&self) -> u32 {
        match self {
            Self::Complete { pieces, .. } => *pieces,
        }
    }

    #[must_use]
    pub const fn bytes(&self) -> u64 {
        match self {
            Self::Complete { bytes, .. } => *bytes,
        }
    }
}

impl SmokeOutcome {
    #[must_use]
    pub const fn piece(&self) -> u32 {
        match self {
            Self::Verified { piece, .. } => *piece,
        }
    }

    #[must_use]
    pub const fn bytes(&self) -> u64 {
        match self {
            Self::Verified { bytes, .. } => *bytes,
        }
    }
}

fn random_peer_id_bytes() -> [u8; 20] {
    rand::random()
}
