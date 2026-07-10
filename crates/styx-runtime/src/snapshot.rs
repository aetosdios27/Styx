use crate::TorrentId;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuntimeSnapshot {
    pub torrents: Vec<TorrentSnapshot>,
    pub peers: Vec<PeerSnapshot>,
    pub events: Vec<RuntimeEvent>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TorrentSnapshot {
    pub id: TorrentId,
    pub name: String,
    pub status: TorrentStatus,
    pub total_bytes: u64,
    pub verified_bytes: u64,
    pub downloaded_bytes: u64,
    pub uploaded_bytes: u64,
    pub down_rate: u64,
    pub up_rate: u64,
    pub peers: u32,
    pub seeds: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PeerSnapshot {
    pub torrent: TorrentId,
    pub source: String,
    pub progress: f32,
    pub down_rate: u64,
    pub up_rate: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TorrentStatus {
    Checking,
    Discovering,
    Downloading,
    Paused,
    Complete,
    Seeding,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RuntimeEvent {
    TorrentAdded {
        torrent: TorrentId,
    },
    TorrentRemoved {
        torrent: TorrentId,
    },
    StateChanged {
        torrent: TorrentId,
        from: TorrentStatus,
        to: TorrentStatus,
    },
    SourceFailed {
        torrent: TorrentId,
        source: String,
        reason: String,
    },
    SourceQuarantined {
        torrent: TorrentId,
        source: String,
    },
    PieceVerified {
        torrent: TorrentId,
        piece: u32,
        bytes: u64,
    },
    ProgressUpdated {
        torrent: TorrentId,
        verified_bytes: u64,
        total_bytes: u64,
    },
    TaskCancelled {
        torrent: TorrentId,
    },
    TaskFailed {
        torrent: TorrentId,
        reason: String,
    },
    TaskCompleted {
        torrent: TorrentId,
    },
    PeerConnected {
        torrent: TorrentId,
        addr: std::net::SocketAddr,
    },
    PeerDisconnected {
        torrent: TorrentId,
        addr: std::net::SocketAddr,
    },
    DhtPeersDiscovered {
        torrent: TorrentId,
        peers: u32,
    },
    DhtAnnounced {
        torrent: TorrentId,
        nodes: u32,
    },
    BlockUploaded {
        torrent: TorrentId,
        peer: std::net::SocketAddr,
        piece: u32,
        offset: u32,
        bytes: u32,
    },
    IntentDeclared {
        torrent: Option<TorrentId>,
        kind: &'static str,
    },
    ValidationStarted,
    ValidationFailed {
        reason: String,
    },
    ValidationSucceeded,
    ExecutionStarted,
    ExecutionSucceeded,
    ExecutionFailed {
        reason: String,
    },
    RollbackStarted,
    RollbackSucceeded,
    RollbackFailed {
        reason: String,
    },
}

impl RuntimeSnapshot {
    #[must_use]
    pub fn torrent_count(&self) -> usize {
        self.torrents.len()
    }

    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }
}

impl TorrentSnapshot {
    #[must_use]
    pub fn new(id: TorrentId, name: impl Into<String>, total_bytes: u64) -> Self {
        Self {
            id,
            name: name.into(),
            status: TorrentStatus::Checking,
            total_bytes,
            verified_bytes: 0,
            downloaded_bytes: 0,
            uploaded_bytes: 0,
            down_rate: 0,
            up_rate: 0,
            peers: 0,
            seeds: 0,
        }
    }

    #[must_use]
    pub fn with_verified_bytes(mut self, bytes: u64) -> Self {
        self.verified_bytes = bytes.min(self.total_bytes);
        self
    }

    #[must_use]
    pub fn with_downloaded_bytes(mut self, bytes: u64) -> Self {
        self.downloaded_bytes = bytes.min(self.total_bytes);
        self
    }

    #[must_use]
    pub fn with_uploaded_bytes(mut self, bytes: u64) -> Self {
        self.uploaded_bytes = bytes;
        self
    }

    #[must_use]
    pub fn progress(&self) -> f32 {
        if self.total_bytes == 0 {
            return 1.0;
        }
        self.verified_bytes as f32 / self.total_bytes as f32
    }

    #[must_use]
    pub fn share_ratio(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        self.uploaded_bytes as f32 / self.total_bytes as f32
    }
}

impl RuntimeEvent {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::TorrentAdded { .. } => "torrent_added",
            Self::TorrentRemoved { .. } => "torrent_removed",
            Self::StateChanged { .. } => "state_changed",
            Self::SourceFailed { .. } => "source_failed",
            Self::SourceQuarantined { .. } => "source_quarantined",
            Self::PieceVerified { .. } => "piece_verified",
            Self::ProgressUpdated { .. } => "progress_updated",
            Self::TaskCancelled { .. } => "task_cancelled",
            Self::TaskFailed { .. } => "task_failed",
            Self::TaskCompleted { .. } => "task_completed",
            Self::PeerConnected { .. } => "peer_connected",
            Self::PeerDisconnected { .. } => "peer_disconnected",
            Self::DhtPeersDiscovered { .. } => "dht_peers_discovered",
            Self::DhtAnnounced { .. } => "dht_announced",
            Self::BlockUploaded { .. } => "block_uploaded",
            Self::IntentDeclared { .. } => "intent_declared",
            Self::ValidationStarted => "validation_started",
            Self::ValidationFailed { .. } => "validation_failed",
            Self::ValidationSucceeded => "validation_succeeded",
            Self::ExecutionStarted => "execution_started",
            Self::ExecutionSucceeded => "execution_succeeded",
            Self::ExecutionFailed { .. } => "execution_failed",
            Self::RollbackStarted => "rollback_started",
            Self::RollbackSucceeded => "rollback_succeeded",
            Self::RollbackFailed { .. } => "rollback_failed",
        }
    }
}
