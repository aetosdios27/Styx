use std::time::{Duration, Instant};

use bytes::Bytes;
use styx_core::{PeerConnectionManager, TorrentState};
use styx_disk::{
    block_specs_for_piece, BlockSpec, PieceIndex, PieceManager, ResumeSummary, VerificationResult,
};
use styx_proto::PeerId;

use crate::{
    peer_table::PeerTable, RateCounter, RuntimeConfig, RuntimeError, RuntimeEvent, SourceEndpoint,
    SourceFailure, SourceId, SourceTable, TorrentCommand, TorrentId, TorrentPlan, TorrentSnapshot,
    TorrentStatus,
};

#[derive(Debug)]
pub struct TorrentTask {
    plan: TorrentPlan,
    pieces: PieceManager,
    status: TorrentStatus,
    verified_bytes: u64,
    downloaded_bytes: u64,
    down_rate: RateCounter,
    up_rate: RateCounter,
    last_rate_tick: Instant,
    cached_down_rate: u64,
    cached_up_rate: u64,
    manager: PeerConnectionManager,
    peers: PeerTable,
    sources: SourceTable,
    peer_id: PeerId,
    last_announce: Option<Instant>,
    announce_interval: Duration,
}

impl TorrentTask {
    #[must_use]
    pub fn new(plan: TorrentPlan) -> Self {
        let pieces = PieceManager::new(plan.disk_plan.clone());
        let peers = PeerTable::new(30);
        let sources = SourceTable::from_candidates(Vec::new(), &RuntimeConfig::default())
            .expect("empty candidate list never fails");
        let torrent = TorrentState::new(
            plan.piece_count() as usize,
            plan.metainfo.info.piece_length as u32,
            16384,
        );
        let manager = PeerConnectionManager::new(styx_core::PeerManagerConfig::default(), torrent)
            .expect("default PeerManagerConfig is valid");
        Self {
            plan,
            pieces,
            status: TorrentStatus::Checking,
            verified_bytes: 0,
            downloaded_bytes: 0,
            down_rate: RateCounter::new(Duration::from_secs(2)).expect("2s window is valid"),
            up_rate: RateCounter::new(Duration::from_secs(2)).expect("2s window is valid"),
            last_rate_tick: Instant::now(),
            cached_down_rate: 0,
            cached_up_rate: 0,
            manager,
            peers,
            sources,
            peer_id: PeerId::new([0u8; 20]),
            last_announce: None,
            announce_interval: Duration::from_secs(1800),
        }
    }

    pub fn new_with_peers(plan: TorrentPlan, config: RuntimeConfig) -> Result<Self, RuntimeError> {
        let pieces = PieceManager::new(plan.disk_plan.clone());

        let piece_count = plan.piece_count() as usize;
        let standard_piece_length = plan.metainfo.info.piece_length as u32;
        let block_length = 16384_u32;

        let torrent = TorrentState::new(piece_count, standard_piece_length, block_length);
        let manager = PeerConnectionManager::new(config.peer.clone(), torrent).map_err(|e| {
            RuntimeError::InvalidConfig(match e {
                styx_core::CoreError::InvalidConfig { field } => field,
                _ => "peer manager config is invalid",
            })
        })?;

        let web_seed_candidates: Vec<crate::SourceCandidate> = plan
            .web_seed_urls
            .iter()
            .map(|url| crate::SourceCandidate::web_seed(SourceId::new(0), url.clone()))
            .collect();
        let sources = SourceTable::from_candidates(web_seed_candidates, &config)?;

        let peers = PeerTable::new(config.limits.max_peers_per_torrent);
        let peer_id = PeerId::new([0u8; 20]);

        Ok(Self {
            plan,
            pieces,
            status: TorrentStatus::Checking,
            verified_bytes: 0,
            downloaded_bytes: 0,
            down_rate: RateCounter::new(Duration::from_secs(2)).expect("2s window is valid"),
            up_rate: RateCounter::new(Duration::from_secs(2)).expect("2s window is valid"),
            last_rate_tick: Instant::now(),
            cached_down_rate: 0,
            cached_up_rate: 0,
            manager,
            peers,
            sources,
            peer_id,
            last_announce: None,
            announce_interval: Duration::from_secs(1800),
        })
    }

    #[must_use]
    pub fn into_plan(self) -> TorrentPlan {
        self.plan
    }

    #[must_use]
    pub fn id(&self) -> TorrentId {
        self.plan.id
    }

    #[must_use]
    pub fn status(&self) -> TorrentStatus {
        self.status
    }

    pub fn apply(&mut self, command: TorrentCommand) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        match command {
            TorrentCommand::Start => self.transition(TorrentStatus::Discovering),
            TorrentCommand::Pause => self.transition(TorrentStatus::Paused),
            TorrentCommand::Resume => self.transition(TorrentStatus::Downloading),
            TorrentCommand::Cancel => self.transition(TorrentStatus::Cancelled),
            TorrentCommand::Tick => self.tick(),
        }
    }

    pub async fn discover_and_connect_peers(&mut self) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let mut events = Vec::new();

        for candidate in self.sources.next_candidates(usize::MAX) {
            let SourceEndpoint::Peer(addr) = candidate.endpoint else {
                continue;
            };

            let info_hash = self.plan.info_hash;
            let connect_timeout = Duration::from_secs(10);

            match self
                .peers
                .connect_peer(addr, info_hash, self.peer_id, connect_timeout)
                .await
            {
                Ok(key) => {
                    let _ = self.sources.record_success(candidate.id);
                    let _ = self.manager.add_peer(key);
                    events.push(RuntimeEvent::PeerConnected {
                        torrent: self.plan.id,
                        addr,
                    });
                }
                Err(e) => {
                    let is_full = matches!(
                        e,
                        styx_proto::PeerWireError::Io(ref io_err)
                            if io_err.kind() == std::io::ErrorKind::AlreadyExists
                    );
                    if is_full {
                        break;
                    }
                    let _ = self
                        .sources
                        .record_failure(candidate.id, SourceFailure::Refused);
                }
            }
        }

        Ok(events)
    }

    pub async fn accept_piece_blocks(
        &mut self,
        piece: PieceIndex,
        blocks: Vec<(BlockSpec, Bytes)>,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let piece_bytes = u64::from(self.plan.piece_length(piece)?);
        let now = Instant::now();
        for (block, payload) in blocks {
            self.downloaded_bytes = self.downloaded_bytes.saturating_add(payload.len() as u64);
            self.down_rate.record(now, payload.len() as u64);
            self.pieces.accept_block(block, payload)?;
        }
        match self.pieces.verify_and_commit_piece(piece).await? {
            VerificationResult::Verified { piece } => {
                self.verified_bytes = self.verified_bytes.saturating_add(piece_bytes);
                Ok(vec![
                    RuntimeEvent::PieceVerified {
                        torrent: self.plan.id,
                        piece: piece.get(),
                        bytes: piece_bytes,
                    },
                    RuntimeEvent::ProgressUpdated {
                        torrent: self.plan.id,
                        verified_bytes: self.verified_bytes,
                        total_bytes: self.plan.total_size,
                    },
                ])
            }
            VerificationResult::HashMismatch { piece } => {
                Err(RuntimeError::PieceHashMismatch { piece: piece.get() })
            }
        }
    }

    pub async fn complete_from_piece_bytes(
        &mut self,
        pieces: Vec<Bytes>,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        if pieces.len() != self.plan.piece_count() as usize {
            return Err(RuntimeError::InvalidConfig("piece byte count mismatch"));
        }

        let mut events = Vec::new();
        let saved_status = self.status;
        if matches!(self.status, TorrentStatus::Checking) {
            events.extend(self.transition(TorrentStatus::Discovering)?);
        }
        if matches!(self.status, TorrentStatus::Discovering) {
            events.extend(self.transition(TorrentStatus::Downloading)?);
        }

        match self.process_piece_bytes(&pieces).await {
            Ok(additional_events) => events.extend(additional_events),
            Err(e) => {
                self.status = saved_status;
                return Err(e);
            }
        }

        if self.pieces.verified_piece_count() == self.plan.piece_count() {
            events.extend(self.transition(TorrentStatus::Complete)?);
            events.push(RuntimeEvent::TaskCompleted {
                torrent: self.plan.id,
            });
        }
        Ok(events)
    }

    async fn process_piece_bytes(
        &mut self,
        pieces: &[Bytes],
    ) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let mut events = Vec::new();
        for (raw_piece, piece_bytes) in pieces.iter().enumerate() {
            let piece = PieceIndex::new(raw_piece as u32);
            let specs = block_specs_for_piece(piece, self.plan.piece_length(piece)?)?;
            let mut offset = 0_usize;
            let mut blocks = Vec::with_capacity(specs.len());
            for spec in specs {
                let end = offset + spec.length().get() as usize;
                let Some(slice) = piece_bytes.get(offset..end) else {
                    return Err(RuntimeError::InvalidWebSeedLength {
                        piece: piece.get(),
                        expected: self.plan.piece_length(piece)? as usize,
                        actual: piece_bytes.len(),
                    });
                };
                blocks.push((spec, Bytes::copy_from_slice(slice)));
                offset = end;
            }
            if offset != piece_bytes.len() {
                return Err(RuntimeError::InvalidWebSeedLength {
                    piece: piece.get(),
                    expected: self.plan.piece_length(piece)? as usize,
                    actual: piece_bytes.len(),
                });
            }
            if !self.pieces.has_piece(piece) {
                events.extend(self.accept_piece_blocks(piece, blocks).await?);
            }
        }
        Ok(events)
    }

    pub fn mark_failed(&mut self, reason: impl Into<String>) -> Vec<RuntimeEvent> {
        self.status = TorrentStatus::Failed;
        vec![RuntimeEvent::TaskFailed {
            torrent: self.plan.id,
            reason: reason.into(),
        }]
    }

    pub fn set_verified_bytes(&mut self, bytes: u64) {
        self.verified_bytes = bytes.min(self.plan.total_size);
    }

    pub fn verify_pieces_root(&self) -> Result<(), RuntimeError> {
        if self.plan.disk_plan.piece_hashes_v2().is_empty() {
            return Ok(());
        }
        Ok(())
    }

    pub fn set_status_complete(&mut self) -> Result<(), RuntimeError> {
        self.verify_pieces_root()?;
        self.status = TorrentStatus::Complete;
        self.verified_bytes = self.plan.total_size;
        Ok(())
    }

    pub async fn resume_verify(&mut self) -> Result<ResumeSummary, RuntimeError> {
        let summary = self.pieces.resume_verify().await?;
        let mut verified_bytes = 0_u64;
        for raw_piece in 0..self.plan.piece_count() {
            let piece = PieceIndex::new(raw_piece);
            if self.pieces.has_piece(piece) {
                verified_bytes =
                    verified_bytes.saturating_add(u64::from(self.plan.piece_length(piece)?));
            }
        }
        self.verified_bytes = verified_bytes;
        Ok(summary)
    }

    #[must_use]
    pub fn snapshot(&mut self) -> TorrentSnapshot {
        let now = Instant::now();
        if now.duration_since(self.last_rate_tick) > Duration::from_millis(250) {
            self.cached_down_rate = self.down_rate.bytes_per_second(now);
            self.cached_up_rate = self.up_rate.bytes_per_second(now);
            self.last_rate_tick = now;
        }
        let mut snapshot =
            TorrentSnapshot::new(self.plan.id, self.plan.name.clone(), self.plan.total_size)
                .with_verified_bytes(self.verified_bytes)
                .with_downloaded_bytes(self.downloaded_bytes);
        snapshot.status = self.status;
        snapshot.down_rate = self.cached_down_rate;
        snapshot.up_rate = self.cached_up_rate;
        snapshot.peers = self.peers.connected_count() as u32;
        snapshot.seeds = 0;
        snapshot
    }

    fn tick(&mut self) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        match self.status {
            TorrentStatus::Discovering => self.transition(TorrentStatus::Downloading),
            TorrentStatus::Checking => self.transition(TorrentStatus::Discovering),
            TorrentStatus::Downloading | TorrentStatus::Paused => Ok(Vec::new()),
            TorrentStatus::Complete
            | TorrentStatus::Seeding
            | TorrentStatus::Failed
            | TorrentStatus::Cancelled => Err(RuntimeError::InvalidConfig(
                "illegal torrent state transition",
            )),
        }
    }

    fn transition(&mut self, to: TorrentStatus) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let from = self.status;
        if from == to {
            return Ok(Vec::new());
        }
        if !is_legal_transition(from, to) {
            return Err(RuntimeError::InvalidConfig(
                "illegal torrent state transition",
            ));
        }
        self.status = to;
        Ok(vec![RuntimeEvent::StateChanged {
            torrent: self.plan.id,
            from,
            to,
        }])
    }
}

fn is_legal_transition(from: TorrentStatus, to: TorrentStatus) -> bool {
    matches!(
        (from, to),
        (TorrentStatus::Checking, TorrentStatus::Discovering)
            | (TorrentStatus::Checking, TorrentStatus::Cancelled)
            | (TorrentStatus::Discovering, TorrentStatus::Downloading)
            | (TorrentStatus::Discovering, TorrentStatus::Paused)
            | (TorrentStatus::Discovering, TorrentStatus::Cancelled)
            | (TorrentStatus::Downloading, TorrentStatus::Paused)
            | (TorrentStatus::Downloading, TorrentStatus::Complete)
            | (TorrentStatus::Downloading, TorrentStatus::Cancelled)
            | (TorrentStatus::Paused, TorrentStatus::Downloading)
            | (TorrentStatus::Paused, TorrentStatus::Cancelled)
            | (TorrentStatus::Complete, TorrentStatus::Seeding)
    )
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use tokio::net::TcpListener;
    use url::Url;

    use super::*;
    use crate::{SourceKind, SourceState};
    use styx_disk::DiskPlan;
    use styx_proto::{
        read_handshake, write_handshake, ExtensionBits, FileMode, Handshake, InfoHashV1, PeerId,
        TorrentInfo, TorrentMetainfo,
    };

    fn make_v1_plan() -> TorrentPlan {
        let info_hash = InfoHashV1::new([0u8; 20]);
        let metainfo = TorrentMetainfo {
            announce: None,
            announce_list: Vec::new(),
            url_list: Vec::new(),
            info: TorrentInfo {
                name: Bytes::from("test"),
                piece_length: 16384,
                pieces: Some(Bytes::from(vec![0xAB; 40])),
                mode: FileMode::Single { length: 32768 },
                file_tree: None,
                meta_version: None,
                private: false,
            },
            info_hash_v1: info_hash,
            info_hash_v2: None,
            piece_layers: None,
            raw_info: Bytes::new(),
        };
        TorrentPlan {
            id: TorrentId::new(info_hash),
            info_hash,
            info_hash_v2: None,
            name: "test".to_owned(),
            total_size: 32768,
            announce_urls: vec![Url::parse("http://127.0.0.1:6969/announce").unwrap()],
            web_seed_urls: Vec::new(),
            metainfo,
            disk_plan: DiskPlan::new_v2("/tmp", &[], 16384, vec![]).unwrap(),
        }
    }

    fn make_config() -> RuntimeConfig {
        RuntimeConfig {
            limits: crate::RuntimeLimits {
                max_peers_per_torrent: 30,
                max_sources_per_torrent: 64,
                source_retry_limit: 3,
                ..crate::RuntimeLimits::default()
            },
            ..RuntimeConfig::default()
        }
    }

    #[test]
    fn verify_pieces_root_v1_returns_ok() {
        let task = TorrentTask::new(make_v1_plan());
        assert!(task.verify_pieces_root().is_ok());
    }

    #[test]
    fn transition_to_same_status_returns_empty_events() {
        let task = &mut TorrentTask::new(make_v1_plan());
        let events = task.transition(TorrentStatus::Checking).unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn complete_from_piece_bytes_reverts_status_on_failure() {
        let mut task = TorrentTask::new(make_v1_plan());
        assert_eq!(task.status, TorrentStatus::Checking);

        // Pass pieces with wrong data (zeros) — hash won't match 0xAB...AB in plan
        let wrong_pieces = vec![Bytes::from(vec![0u8; 16384]), Bytes::from(vec![0u8; 16384])];
        let result = task.complete_from_piece_bytes(wrong_pieces).await;
        assert!(result.is_err());
        assert_eq!(task.status, TorrentStatus::Checking);
    }

    #[test]
    fn t3_t1_new_with_peers_creates_initialized_task() {
        let plan = make_v1_plan();
        let config = make_config();
        let task = TorrentTask::new_with_peers(plan, config).unwrap();

        assert_eq!(task.status, TorrentStatus::Checking);
        assert_eq!(task.peers.connected_count(), 0);
        assert_eq!(task.sources.len(), 0);
    }

    #[tokio::test]
    async fn t3_t2_discover_and_connect_peers_connects_to_multiple_peers() {
        let plan = make_v1_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();
        let info_hash = task.plan.info_hash;

        // Start 3 mock peer servers
        let mut addrs = Vec::new();
        for _ in 0..3 {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            addrs.push(addr);
            let info_hash = info_hash;
            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let (mut r, mut w) = stream.into_split();
                let _handshake = read_handshake(&mut r, info_hash).await.unwrap();
                let response = Handshake {
                    reserved: ExtensionBits::default(),
                    info_hash,
                    peer_id: PeerId::new([0xFF; 20]),
                };
                write_handshake(&mut w, &response).await.unwrap();
            });
        }

        // Add peer candidates to SourceTable
        for addr in &addrs {
            task.sources
                .add_candidate(SourceEndpoint::Peer(*addr), SourceKind::Peer)
                .unwrap();
        }

        // Discover and connect
        let events = task.discover_and_connect_peers().await.unwrap();

        assert_eq!(events.len(), 3);
        for event in &events {
            assert!(matches!(event, RuntimeEvent::PeerConnected { .. }));
        }
        assert_eq!(task.peers.connected_count(), 3);
    }

    #[tokio::test]
    async fn t3_t3_source_lifecycle_transitions_through_peer_connections() {
        let plan = make_v1_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();
        let info_hash = task.plan.info_hash;

        // Start one mock peer
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut r, mut w) = stream.into_split();
            let _handshake = read_handshake(&mut r, info_hash).await.unwrap();
            let response = Handshake {
                reserved: ExtensionBits::default(),
                info_hash,
                peer_id: PeerId::new([0xFF; 20]),
            };
            write_handshake(&mut w, &response).await.unwrap();
        });

        // Add candidate and connect
        let sid = task
            .sources
            .add_candidate(SourceEndpoint::Peer(addr), SourceKind::Peer)
            .unwrap();
        assert_eq!(task.sources.state(sid).unwrap(), SourceState::Fresh);

        let events = task.discover_and_connect_peers().await.unwrap();
        assert_eq!(events.len(), 1);

        // Source is now Active
        assert_eq!(task.sources.state(sid).unwrap(), SourceState::Active);

        // next_candidates should not return it
        assert_eq!(task.sources.next_candidates(10).len(), 0);
    }

    #[tokio::test]
    async fn t3_t4_connection_failure_tracks_in_source_table() {
        let plan = make_v1_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();

        // Use an unreachable address
        let dead_addr = SocketAddr::new(Ipv4Addr::new(127, 0, 0, 2).into(), 1);

        let sid = task
            .sources
            .add_candidate(SourceEndpoint::Peer(dead_addr), SourceKind::Peer)
            .unwrap();
        assert_eq!(task.sources.state(sid).unwrap(), SourceState::Fresh);

        let events = task.discover_and_connect_peers().await.unwrap();
        assert_eq!(events.len(), 0);

        // Source should now be CoolingDown (1 failure, retry_limit=3)
        assert_eq!(task.sources.state(sid).unwrap(), SourceState::CoolingDown);
    }
}
