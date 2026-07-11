use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use bytes::Bytes;
use styx_core::{
    BlockRequest, PeerAction, PeerConnectionManager, PeerIdentityManager, PeerKey, PrivacyConfig,
    TorrentState,
};
use styx_disk::{
    block_specs_for_piece, BlockSpec, PieceCompletion, PieceIndex, PieceManager, ResumeSummary,
    VerificationResult,
};
use styx_proto::{
    decode_extension_handshake, decode_pex_message, encode_extension_handshake, ExtensionHandshake,
    PeerId, PeerMessage,
};

use styx_tracker::HttpTrackerClient;

use crate::{
    peer_table::PeerTable, DiscoveryPolicy, RateCounter, RuntimeConfig, RuntimeError, RuntimeEvent,
    SourceEndpoint, SourceFailure, SourceId, SourceTable, TorrentCommand, TorrentId, TorrentPlan,
    TorrentSnapshot, TorrentStatus,
};

const LOCAL_PEX_MESSAGE_ID: u8 = 1;

#[derive(Debug)]
pub struct TorrentTask {
    plan: TorrentPlan,
    pieces: PieceManager,
    status: TorrentStatus,
    verified_bytes: u64,
    downloaded_bytes: u64,
    uploaded_bytes: u64,
    down_rate: RateCounter,
    up_rate: RateCounter,
    last_rate_tick: Instant,
    cached_down_rate: u64,
    cached_up_rate: u64,
    manager: PeerConnectionManager,
    peers: PeerTable,
    sources: SourceTable,
    peer_id: PeerId,
    tracker: HttpTrackerClient,
    seed_after_complete: bool,
    pending_verify: Vec<PieceIndex>,
    pending_verify_peers: BTreeMap<PieceIndex, Vec<styx_core::PeerKey>>,
    pending_verify_peer_addrs: BTreeMap<PieceIndex, Vec<SocketAddr>>,
    last_announce: Option<Instant>,
    announce_interval: Duration,
    remote_pex_ids: HashMap<PeerKey, u8>,
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
            uploaded_bytes: 0,
            down_rate: RateCounter::new(Duration::from_secs(2)).expect("2s window is valid"),
            up_rate: RateCounter::new(Duration::from_secs(2)).expect("2s window is valid"),
            last_rate_tick: Instant::now(),
            cached_down_rate: 0,
            cached_up_rate: 0,
            manager,
            peers,
            sources,
            peer_id: fresh_peer_id(),
            last_announce: None,
            announce_interval: Duration::from_secs(1800),
            tracker: HttpTrackerClient::new(512 * 1024),
            seed_after_complete: RuntimeConfig::default().seed_policy.seed_after_complete,
            pending_verify: Vec::new(),
            pending_verify_peers: BTreeMap::new(),
            pending_verify_peer_addrs: BTreeMap::new(),
            remote_pex_ids: HashMap::new(),
        }
    }

    pub fn new_with_peers(plan: TorrentPlan, config: RuntimeConfig) -> Result<Self, RuntimeError> {
        Self::new_with_peers_and_peer_id(plan, config, fresh_peer_id())
    }

    pub(crate) fn new_with_peers_and_peer_id(
        plan: TorrentPlan,
        config: RuntimeConfig,
        peer_id: PeerId,
    ) -> Result<Self, RuntimeError> {
        let pieces = PieceManager::new(plan.disk_plan.clone());

        let piece_count = plan.piece_count() as usize;
        let standard_piece_length = plan.metainfo.info.piece_length as u32;
        let block_length = 16384_u32;

        let torrent = TorrentState::new(piece_count, standard_piece_length, block_length);
        let manager = PeerConnectionManager::new(config.peer, torrent).map_err(|e| {
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
        Ok(Self {
            plan,
            pieces,
            status: TorrentStatus::Checking,
            verified_bytes: 0,
            downloaded_bytes: 0,
            uploaded_bytes: 0,
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
            tracker: HttpTrackerClient::new(512 * 1024),
            seed_after_complete: config.seed_policy.seed_after_complete,
            pending_verify: Vec::new(),
            pending_verify_peers: BTreeMap::new(),
            pending_verify_peer_addrs: BTreeMap::new(),
            remote_pex_ids: HashMap::new(),
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

    pub fn add_dht_peers(&mut self, peers: impl IntoIterator<Item = SocketAddr>) -> usize {
        if !DiscoveryPolicy::from_metainfo(&self.plan.metainfo).dht_allowed() {
            return 0;
        }
        let mut added = 0;
        for peer in peers {
            if peer.port() == 0 || peer.ip().is_unspecified() {
                continue;
            }
            if self.sources.add_dht_peer(peer).is_ok() {
                added += 1;
            }
        }
        added
    }

    pub fn ingest_pex_peers(&mut self, peers: impl IntoIterator<Item = SocketAddr>) -> usize {
        if !DiscoveryPolicy::from_metainfo(&self.plan.metainfo).pex_allowed() {
            return 0;
        }
        let mut added = 0;
        let mut ipv4 = 0;
        let mut ipv6 = 0;
        for peer in peers {
            let family_count = if peer.is_ipv4() { &mut ipv4 } else { &mut ipv6 };
            if *family_count == styx_proto::MAX_PEX_CONTACTS_PER_FAMILY {
                continue;
            }
            *family_count += 1;
            if is_public_pex_endpoint(peer) && self.sources.add_pex_peer(peer).is_ok() {
                added += 1;
            }
        }
        added
    }

    pub fn add_lsd_peer(&mut self, peer: SocketAddr) -> bool {
        DiscoveryPolicy::from_metainfo(&self.plan.metainfo).lsd_allowed()
            && peer.port() != 0
            && !peer.ip().is_unspecified()
            && self.sources.add_lsd_peer(peer).is_ok()
    }

    #[must_use]
    pub(crate) fn dht_announce_target(&self) -> Option<styx_dht::InfoHash> {
        let active = matches!(
            self.status,
            TorrentStatus::Discovering | TorrentStatus::Downloading | TorrentStatus::Seeding
        );
        (active && DiscoveryPolicy::from_metainfo(&self.plan.metainfo).dht_allowed())
            .then(|| styx_dht::InfoHash::new(*self.plan.info_hash.as_bytes()))
    }

    pub(crate) fn lsd_announce_target(&self) -> Option<styx_proto::InfoHashV1> {
        let active = matches!(
            self.status,
            TorrentStatus::Discovering | TorrentStatus::Downloading | TorrentStatus::Seeding
        );
        (active && DiscoveryPolicy::from_metainfo(&self.plan.metainfo).lsd_allowed())
            .then_some(self.plan.info_hash)
    }

    pub fn apply(&mut self, command: TorrentCommand) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        match command {
            TorrentCommand::Start => self.transition(TorrentStatus::Discovering),
            TorrentCommand::Pause => self.transition(TorrentStatus::Paused),
            TorrentCommand::Resume if self.verified_bytes == self.plan.total_size => {
                self.transition(TorrentStatus::Seeding)
            }
            TorrentCommand::Resume => self.transition(TorrentStatus::Downloading),
            TorrentCommand::Cancel => self.transition(TorrentStatus::Cancelled),
            TorrentCommand::Tick => self.tick(),
        }
    }

    pub async fn discover_and_connect_peers(&mut self) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let mut events = Vec::new();

        // Re-announce to tracker if interval elapsed
        let now = Instant::now();
        let needs_peers = self.peers.connected_count() == 0;
        let has_trackers = !self.plan.announce_urls.is_empty();
        let should_announce = has_trackers
            && match self.last_announce {
                Some(last) if !needs_peers => {
                    now.duration_since(last) >= self.announce_interval / 2
                }
                Some(last) => now.duration_since(last) >= Duration::from_secs(5),
                None => true,
            };

        if should_announce {
            self.rotate_peer_identity();
            let request = crate::tracker::build_plan_announce_request(
                &self.plan,
                self.peer_id,
                self.verified_bytes,
                50,
            );
            for url in &self.plan.announce_urls {
                match self.tracker.announce(url, &request).await {
                    Ok(response) => {
                        self.announce_interval = Duration::from_secs(u64::from(response.interval));
                        for peer in &response.peers {
                            if peer.addr.port() == 0 || peer.addr.ip().is_unspecified() {
                                continue;
                            }
                            let _ = self.sources.add_candidate(
                                SourceEndpoint::Peer(peer.addr),
                                crate::SourceKind::Peer,
                            );
                        }
                    }
                    Err(e) => {
                        events.push(RuntimeEvent::SourceFailed {
                            torrent: self.plan.id,
                            source: url.to_string(),
                            reason: e.to_string(),
                        });
                    }
                }
            }
            self.last_announce = Some(now);
        }

        // Connect fresh peers from SourceTable
        for candidate in self.sources.next_candidates(usize::MAX) {
            let SourceEndpoint::Peer(addr) = candidate.endpoint else {
                continue;
            };

            let info_hash = self.plan.info_hash;
            let connect_timeout = Duration::from_secs(10);
            self.rotate_peer_identity();

            match self
                .peers
                .connect_peer(addr, info_hash, self.peer_id, connect_timeout)
                .await
            {
                Ok(key) => {
                    let _ = self.sources.record_success(candidate.id);
                    let _ = self.manager.add_peer(key);
                    self.advertise_pex(key);
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
            if self.seed_after_complete {
                events.extend(self.start_seeding()?);
            }
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

    pub fn start_seeding(&mut self) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        if self.status == TorrentStatus::Seeding {
            return Ok(Vec::new());
        }
        if self.status != TorrentStatus::Complete {
            return Err(RuntimeError::InvalidConfig(
                "torrent must be complete before seeding",
            ));
        }
        self.transition(TorrentStatus::Seeding)
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
                .with_downloaded_bytes(self.downloaded_bytes)
                .with_uploaded_bytes(self.uploaded_bytes);
        snapshot.status = self.status;
        snapshot.down_rate = self.cached_down_rate;
        snapshot.up_rate = self.cached_up_rate;
        snapshot.peers = self.peers.connected_count() as u32;
        snapshot.seeds = self.manager.seed_count() as u32;
        snapshot
    }

    fn tick(&mut self) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let mut events = self.tick_state()?;

        if self.status != TorrentStatus::Downloading {
            return Ok(events);
        }

        let now = Instant::now();

        // Drain incoming messages from all peers
        let (messages, dead) = self.peers.drain_messages();
        for (key, msg) in messages {
            if self.handle_pex_wire_message(key, &msg) {
                continue;
            }
            if let Ok(actions) = self.manager.handle_message(key, msg, now) {
                events.extend(self.execute_actions(actions));
            }
        }
        for (key, addr) in dead {
            self.peers.remove_peer(key);
            let _ = self.manager.remove_peer(key);
            events.push(RuntimeEvent::PeerDisconnected {
                torrent: self.plan.id,
                addr,
            });
        }

        // Drive policy: choke/unchoke + block requests
        if let Ok(actions) = self.manager.tick(now) {
            events.extend(self.execute_actions(actions));
        }

        Ok(events)
    }

    pub async fn tick_and_verify(&mut self) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let mut events = self.tick()?;
        events.extend(self.verify_completed_pieces().await?);
        if self.status == TorrentStatus::Downloading
            && self.pieces.verified_piece_count() == self.plan.piece_count()
        {
            events.extend(self.transition(TorrentStatus::Complete)?);
            events.push(RuntimeEvent::TaskCompleted {
                torrent: self.plan.id,
            });
            if self.seed_after_complete {
                events.extend(self.start_seeding()?);
            }
        }
        Ok(events)
    }

    pub async fn tick_seed_and_upload(&mut self) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        if self.status != TorrentStatus::Seeding {
            return Ok(Vec::new());
        }

        let mut events = Vec::new();
        let now = Instant::now();
        let (messages, dead) = self.peers.drain_messages();
        for (key, msg) in messages {
            if self.handle_pex_wire_message(key, &msg) {
                continue;
            }
            if let Ok(actions) = self.manager.handle_message(key, msg, now) {
                events.extend(self.execute_actions_with_uploads(actions).await);
            }
        }
        for (key, addr) in dead {
            self.peers.remove_peer(key);
            let _ = self.manager.remove_peer(key);
            events.push(RuntimeEvent::PeerDisconnected {
                torrent: self.plan.id,
                addr,
            });
        }
        if let Ok(actions) = self.manager.tick_seed(now) {
            events.extend(self.execute_actions_with_uploads(actions).await);
        }
        Ok(events)
    }

    fn advertise_pex(&self, peer: PeerKey) {
        if !DiscoveryPolicy::from_metainfo(&self.plan.metainfo).pex_allowed()
            || !self.peers.supports_extended(peer)
        {
            return;
        }
        let mut handshake = ExtensionHandshake::default();
        handshake
            .messages
            .insert("ut_pex".to_owned(), LOCAL_PEX_MESSAGE_ID);
        let _ = self.peers.send_message(
            peer,
            PeerMessage::Extended {
                extension_id: 0,
                payload: Bytes::from(encode_extension_handshake(&handshake)),
            },
        );
    }

    fn rotate_peer_identity(&mut self) -> PeerId {
        self.peer_id = fresh_peer_id();
        self.peer_id
    }

    fn handle_pex_wire_message(&mut self, peer: PeerKey, message: &PeerMessage) -> bool {
        if !DiscoveryPolicy::from_metainfo(&self.plan.metainfo).pex_allowed() {
            return matches!(message, PeerMessage::Extended { .. });
        }
        let PeerMessage::Extended {
            extension_id,
            payload,
        } = message
        else {
            return false;
        };
        if *extension_id == 0 {
            if let Ok(handshake) = decode_extension_handshake(payload) {
                if let Some(id) = handshake.message_id("ut_pex") {
                    self.remote_pex_ids.insert(peer, id);
                } else {
                    self.remote_pex_ids.remove(&peer);
                }
            }
            return true;
        }
        if *extension_id == LOCAL_PEX_MESSAGE_ID && self.remote_pex_ids.contains_key(&peer) {
            if let Ok(pex) = decode_pex_message(payload) {
                self.ingest_pex_peers(pex.added.into_iter().chain(pex.added6));
            }
            return true;
        }
        false
    }

    fn tick_state(&mut self) -> Result<Vec<RuntimeEvent>, RuntimeError> {
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

    fn execute_actions(&mut self, actions: Vec<PeerAction>) -> Vec<RuntimeEvent> {
        let mut events = Vec::new();
        for action in actions {
            match action {
                PeerAction::SendMessage { peer, message } => {
                    let _ = self.peers.send_message(peer, message);
                }
                PeerAction::Disconnect { peer, reason: _ } => {
                    if let Some(addr) = self.peers.peer_addr(peer) {
                        self.peers.remove_peer(peer);
                        let _ = self.manager.remove_peer(peer);
                        events.push(RuntimeEvent::PeerDisconnected {
                            torrent: self.plan.id,
                            addr,
                        });
                    }
                }
                PeerAction::AcceptBlock {
                    peer,
                    request,
                    bytes,
                } => {
                    let piece_length = self.pieces.plan().piece_length(request.piece).ok();
                    if let Some(pl) = piece_length {
                        if let Ok(block) =
                            BlockSpec::new(request.piece, request.offset, request.length, pl)
                        {
                            if let Ok(completion) = self.pieces.accept_block(block, bytes) {
                                self.pending_verify_peers
                                    .entry(request.piece)
                                    .or_default()
                                    .push(peer);
                                if let Some(addr) = self.peers.peer_addr(peer) {
                                    self.pending_verify_peer_addrs
                                        .entry(request.piece)
                                        .or_default()
                                        .push(addr);
                                }
                                if matches!(completion, PieceCompletion::Complete { .. }) {
                                    self.pending_verify.push(request.piece);
                                }
                            }
                        }
                    }
                }
                PeerAction::CancelDuplicate { .. } | PeerAction::RecordInterest { .. } => {}
                PeerAction::ServeBlock { .. } => {}
            }
        }
        events
    }

    pub async fn execute_actions_with_uploads(
        &mut self,
        actions: Vec<PeerAction>,
    ) -> Vec<RuntimeEvent> {
        let mut events = Vec::new();
        for action in actions {
            match action {
                PeerAction::ServeBlock { peer, request } => {
                    let Some(addr) = self.peers.peer_addr(peer) else {
                        continue;
                    };
                    let piece_length = match self.pieces.plan().piece_length(request.piece) {
                        Ok(length) => length,
                        Err(_) => continue,
                    };
                    let block = match BlockSpec::new(
                        request.piece,
                        request.offset,
                        request.length,
                        piece_length,
                    ) {
                        Ok(block) => block,
                        Err(_) => continue,
                    };
                    let bytes = match self.pieces.read_verified_block(block).await {
                        Ok(bytes) => bytes,
                        Err(_) => {
                            self.peers.remove_peer(peer);
                            let _ = self.manager.remove_peer(peer);
                            events.push(RuntimeEvent::PeerDisconnected {
                                torrent: self.plan.id,
                                addr,
                            });
                            continue;
                        }
                    };
                    let byte_count = bytes.len() as u64;
                    let Ok(uploaded_len) = u32::try_from(byte_count) else {
                        continue;
                    };
                    if self
                        .peers
                        .send_message(
                            peer,
                            PeerMessage::Piece {
                                index: request.piece.get(),
                                begin: request.offset.get(),
                                block: bytes,
                            },
                        )
                        .is_ok()
                    {
                        let now = Instant::now();
                        self.uploaded_bytes = self.uploaded_bytes.saturating_add(byte_count);
                        self.up_rate.record(now, byte_count);
                        let _ = self.manager.record_uploaded(peer, byte_count, now);
                        events.push(RuntimeEvent::BlockUploaded {
                            torrent: self.plan.id,
                            peer: addr,
                            piece: request.piece.get(),
                            offset: request.offset.get(),
                            bytes: uploaded_len,
                        });
                    }
                }
                other => {
                    events.extend(self.execute_actions(vec![other]));
                }
            }
        }
        events
    }

    pub async fn verify_completed_pieces(&mut self) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let mut events = Vec::new();
        let mut pieces = std::mem::take(&mut self.pending_verify);
        pieces.sort_unstable();
        pieces.dedup();
        for piece in pieces {
            match self.pieces.verify_and_commit_piece(piece).await {
                Ok(VerificationResult::Verified { piece: _ }) => {
                    self.pending_verify_peers.remove(&piece);
                    self.pending_verify_peer_addrs.remove(&piece);
                    let bytes = self.pieces.plan().piece_length(piece).unwrap_or(0) as u64;
                    self.verified_bytes = self.verified_bytes.saturating_add(bytes);
                    let mut cleanup_actions = Vec::new();
                    for block in block_specs_for_piece(piece, bytes as u32)? {
                        cleanup_actions.extend(self.manager.mark_piece_verified(
                            BlockRequest::new(block.piece(), block.offset(), block.length()),
                        ));
                    }
                    events.extend(self.execute_actions(cleanup_actions));
                    events.push(RuntimeEvent::PieceVerified {
                        torrent: self.plan.id,
                        piece: piece.get(),
                        bytes,
                    });
                    events.push(RuntimeEvent::ProgressUpdated {
                        torrent: self.plan.id,
                        verified_bytes: self.verified_bytes,
                        total_bytes: self.total_bytes(),
                    });
                }
                Ok(VerificationResult::HashMismatch { piece }) => {
                    let peers = self.pending_verify_peers.remove(&piece).unwrap_or_default();
                    let mut addrs = self
                        .pending_verify_peer_addrs
                        .remove(&piece)
                        .unwrap_or_default();
                    for peer in peers {
                        if let Some(addr) = self.peers.peer_addr(peer) {
                            addrs.push(addr);
                            self.peers.remove_peer(peer);
                            let _ = self.manager.remove_peer(peer);
                            events.push(RuntimeEvent::PeerDisconnected {
                                torrent: self.plan.id,
                                addr,
                            });
                        }
                    }
                    addrs.sort_unstable();
                    addrs.dedup();
                    for addr in addrs {
                        if let Some(source) =
                            self.sources.id_for_endpoint(&SourceEndpoint::Peer(addr))
                        {
                            let _ = self
                                .sources
                                .record_failure(source, SourceFailure::CorruptData);
                        }
                        events.push(RuntimeEvent::SourceQuarantined {
                            torrent: self.plan.id,
                            source: addr.to_string(),
                        });
                    }
                }
                Err(e) => {
                    return Err(RuntimeError::from(e));
                }
            }
        }
        Ok(events)
    }

    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.plan.total_size
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

fn is_public_pex_endpoint(endpoint: SocketAddr) -> bool {
    if endpoint.port() == 0 {
        return false;
    }
    match endpoint.ip() {
        std::net::IpAddr::V4(ip) => {
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_documentation()
                || ip.octets()[0] == 0
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
                || (ip.octets()[0] == 198 && (18..=19).contains(&ip.octets()[1]))
                || ip.octets()[0] >= 240)
        }
        std::net::IpAddr::V6(ip) => {
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || (ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8))
        }
    }
}

fn fresh_peer_id() -> PeerId {
    static IDENTITIES: OnceLock<Mutex<PeerIdentityManager>> = OnceLock::new();
    IDENTITIES
        .get_or_init(|| {
            Mutex::new(
                PeerIdentityManager::new(PrivacyConfig::default())
                    .expect("default privacy configuration is valid"),
            )
        })
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .generate(&mut rand::rng())
        .expect("cryptographic peer identity generation should not exhaust")
        .peer_id
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
            | (TorrentStatus::Paused, TorrentStatus::Seeding)
            | (TorrentStatus::Paused, TorrentStatus::Cancelled)
            | (TorrentStatus::Complete, TorrentStatus::Seeding)
            | (TorrentStatus::Seeding, TorrentStatus::Paused)
            | (TorrentStatus::Seeding, TorrentStatus::Cancelled)
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::{Ipv4Addr, SocketAddr};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::{SourceKind, SourceState};
    use sha1::{Digest, Sha1};
    use styx_disk::{BlockLength, BlockOffset, DiskPlan};
    use styx_proto::{
        read_handshake, read_message, write_handshake, write_message, BencodeValue, ExtensionBits,
        FileMode, Handshake, InfoHashV1, PeerId, PeerMessage, TorrentInfo, TorrentMetainfo,
        DEFAULT_MAX_PEER_FRAME_LEN,
    };

    const TEST_INFO_HASH: [u8; 20] = [0u8; 20];
    const VERIFIABLE_INFO_HASH: [u8; 20] = [1u8; 20];

    fn make_test_plan(info_hash_bytes: [u8; 20], total_size: u64, pieces: Vec<u8>) -> TorrentPlan {
        let info_hash = InfoHashV1::new(info_hash_bytes);
        let metainfo = TorrentMetainfo {
            announce: None,
            announce_list: Vec::new(),
            url_list: Vec::new(),
            info: TorrentInfo {
                name: Bytes::from("test"),
                piece_length: 16384,
                pieces: Some(Bytes::from(pieces)),
                mode: FileMode::Single { length: total_size },
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
            total_size,
            announce_urls: Vec::new(),
            web_seed_urls: Vec::new(),
            disk_plan: DiskPlan::from_metainfo(&metainfo, unique_test_root()).unwrap(),
            metainfo,
        }
    }

    fn unique_test_root() -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "styx-runtime-task-test-{}-{nonce}",
            std::process::id()
        ))
    }

    fn make_v1_plan() -> TorrentPlan {
        make_test_plan(TEST_INFO_HASH, 32768, vec![0xAB; 40])
    }

    fn make_verifiable_plan() -> TorrentPlan {
        let piece_data = [0x42u8; 16384];
        let hash: [u8; 20] = Sha1::digest(piece_data).into();
        make_test_plan(VERIFIABLE_INFO_HASH, 16384, hash.to_vec())
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

    #[test]
    fn production_tasks_use_distinct_nonzero_peer_identities() {
        let first = TorrentTask::new(make_v1_plan());
        let second = TorrentTask::new(make_v1_plan());

        assert_ne!(first.peer_id, PeerId::new([0; 20]));
        assert_ne!(second.peer_id, PeerId::new([0; 20]));
        assert_ne!(first.peer_id, second.peer_id);
    }

    #[test]
    fn announce_and_reconnect_rotations_never_reuse_peer_identity() {
        let mut task = TorrentTask::new(make_v1_plan());
        let mut identities = std::collections::HashSet::from([task.peer_id]);

        for _ in 0..128 {
            let identity = task.rotate_peer_identity();
            assert_ne!(identity, PeerId::new([0; 20]));
            assert!(identities.insert(identity));
        }
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

    #[tokio::test]
    async fn t4_t1_tick_processes_messages_and_drives_policy() {
        let plan = make_v1_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();
        let info_hash = task.plan.info_hash;

        // Transition to Downloading
        task.apply(TorrentCommand::Start).unwrap();
        task.tick().unwrap();
        assert_eq!(task.status, TorrentStatus::Downloading);

        // Start a mock seed peer
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let mock_msgs: std::sync::Arc<std::sync::Mutex<Vec<PeerMessage>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let msgs = mock_msgs.clone();

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

            // Send bitfield (2 pieces available) and unchoke
            write_message(
                &mut w,
                &PeerMessage::Bitfield {
                    bytes: vec![0xC0].into(),
                },
            )
            .await
            .unwrap();
            write_message(&mut w, &PeerMessage::Unchoke).await.unwrap();

            // Collect messages from the client until read error
            while let Ok(msg) = read_message(&mut r, DEFAULT_MAX_PEER_FRAME_LEN).await {
                msgs.lock().unwrap().push(msg);
            }
        });

        // Connect peer via discover_and_connect_peers
        task.sources
            .add_candidate(SourceEndpoint::Peer(addr), SourceKind::Peer)
            .unwrap();
        let events = task.discover_and_connect_peers().await.unwrap();
        assert_eq!(events.len(), 1);

        // Give the mock peer time to send bitfield + unchoke and for read_loop to pick them up
        tokio::time::sleep(Duration::from_millis(200)).await;

        // First tick: drain messages → handle_message(Bitfield + Unchoke)
        // Should produce SendMessage(Interested) actions and mark peer unchoked
        let events = task.tick().unwrap();
        assert!(events.is_empty());

        // Wait for write loop to send Interested over TCP
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Second tick: manager.tick() should request blocks since we're interested + unchoked
        let events = task.tick().unwrap();
        assert!(events.is_empty());

        // Wait for write loop to flush
        tokio::time::sleep(Duration::from_millis(200)).await;

        let received = mock_msgs.lock().unwrap();
        assert!(
            !received.is_empty(),
            "expected at least one message from tick"
        );

        let has_interested = received
            .iter()
            .any(|m| matches!(m, PeerMessage::Interested));
        let has_request = received
            .iter()
            .any(|m| matches!(m, PeerMessage::Request { .. }));
        assert!(
            has_interested,
            "mock should have received Interested; got: {received:?}"
        );
        assert!(
            has_request,
            "mock should have received at least one Request; got: {received:?}"
        );
    }

    #[tokio::test]
    async fn t4_t2_tick_handles_peer_disconnect_gracefully() {
        let plan = make_v1_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();
        let info_hash = task.plan.info_hash;

        // Transition to Downloading
        task.apply(TorrentCommand::Start).unwrap();
        task.tick().unwrap();
        assert_eq!(task.status, TorrentStatus::Downloading);

        // Start mock peer that disconnects after handshake
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
            // Drop halves to close the connection
            drop(r);
            drop(w);
        });

        // Connect peer
        task.sources
            .add_candidate(SourceEndpoint::Peer(addr), SourceKind::Peer)
            .unwrap();
        let events = task.discover_and_connect_peers().await.unwrap();
        assert_eq!(events.len(), 1);

        tokio::time::sleep(Duration::from_millis(100)).await;

        // tick should handle gracefully (drain returns empty or processes remaining messages)
        let result = task.tick();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn t4_t3_tick_accepts_piece_and_emits_verified_events() {
        let plan = make_verifiable_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();
        let info_hash = task.plan.info_hash;

        // Transition to Downloading
        task.apply(TorrentCommand::Start).unwrap();
        task.tick().unwrap();
        assert_eq!(task.status, TorrentStatus::Downloading);

        // Start mock seed peer that reads Request then sends Piece
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let piece_data = vec![0x42u8; 16384];

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

            // Send Bitfield (1 piece) + Unchoke
            write_message(
                &mut w,
                &PeerMessage::Bitfield {
                    bytes: vec![0x80].into(),
                },
            )
            .await
            .unwrap();
            write_message(&mut w, &PeerMessage::Unchoke).await.unwrap();

            // Wait for the client to send Interested + Request
            loop {
                match read_message(&mut r, DEFAULT_MAX_PEER_FRAME_LEN).await {
                    Ok(PeerMessage::Request {
                        index: 0,
                        begin: 0,
                        length: 16384,
                    }) => {
                        write_message(
                            &mut w,
                            &PeerMessage::Piece {
                                index: 0,
                                begin: 0,
                                block: piece_data.into(),
                            },
                        )
                        .await
                        .unwrap();
                        break;
                    }
                    Ok(PeerMessage::Interested) => continue,
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }

            // Stay alive
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });

        // Connect peer
        task.sources
            .add_candidate(SourceEndpoint::Peer(addr), SourceKind::Peer)
            .unwrap();
        let events = task.discover_and_connect_peers().await.unwrap();
        assert_eq!(events.len(), 1);

        // Let messages from mock peer arrive
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Tick: drain Bitfield + Unchoke → set interested/unchoked,
        // then manager.tick() → request_blocks → sends Request message to peer
        let _events = task.tick().unwrap();

        // Allow write loop to flush Interested + Request to TCP,
        // mock peer to read Request and respond with Piece,
        // then read_loop to pick it up
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Tick: drain Piece → handle_piece → AcceptBlock → piece completes
        let _events = task.tick().unwrap();

        // Verify the completed piece async
        let events = task.verify_completed_pieces().await.unwrap();

        let has_verified = events
            .iter()
            .any(|e| matches!(e, RuntimeEvent::PieceVerified { .. }));
        let has_progress = events
            .iter()
            .any(|e| matches!(e, RuntimeEvent::ProgressUpdated { .. }));
        assert!(
            has_verified,
            "expected PieceVerified event; got: {events:?}"
        );
        assert!(
            has_progress,
            "expected ProgressUpdated event; got: {events:?}"
        );
    }

    #[tokio::test]
    async fn t7_t1_peer_tick_verifies_completed_piece_without_manual_verify_step() {
        let plan = make_verifiable_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();
        let info_hash = task.plan.info_hash;

        task.apply(TorrentCommand::Start).unwrap();
        task.tick().unwrap();
        assert_eq!(task.status, TorrentStatus::Downloading);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let piece_data = vec![0x42u8; 16384];

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

            write_message(
                &mut w,
                &PeerMessage::Bitfield {
                    bytes: vec![0x80].into(),
                },
            )
            .await
            .unwrap();
            write_message(&mut w, &PeerMessage::Unchoke).await.unwrap();

            loop {
                match read_message(&mut r, DEFAULT_MAX_PEER_FRAME_LEN).await {
                    Ok(PeerMessage::Request {
                        index: 0,
                        begin: 0,
                        length: 16384,
                    }) => {
                        write_message(
                            &mut w,
                            &PeerMessage::Piece {
                                index: 0,
                                begin: 0,
                                block: piece_data.into(),
                            },
                        )
                        .await
                        .unwrap();
                        break;
                    }
                    Ok(PeerMessage::Interested) => continue,
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }

            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });

        task.sources
            .add_candidate(SourceEndpoint::Peer(addr), SourceKind::Peer)
            .unwrap();
        let events = task.discover_and_connect_peers().await.unwrap();
        assert_eq!(events.len(), 1);

        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = task.tick_and_verify().await.unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;
        let events = task.tick_and_verify().await.unwrap();

        assert!(
            events
                .iter()
                .any(|e| matches!(e, RuntimeEvent::PieceVerified { piece: 0, .. })),
            "expected PieceVerified from async peer tick; got: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, RuntimeEvent::ProgressUpdated { .. })),
            "expected ProgressUpdated from async peer tick; got: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, RuntimeEvent::TaskCompleted { .. })),
            "expected TaskCompleted from async peer tick; got: {events:?}"
        );
        assert_eq!(task.status, TorrentStatus::Seeding);
    }

    #[tokio::test]
    async fn t4_t2_corrupt_piece_data_fails_verification_gracefully() {
        let plan = make_verifiable_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();
        let info_hash = task.plan.info_hash;

        task.apply(TorrentCommand::Start).unwrap();
        task.tick().unwrap();
        assert_eq!(task.status, TorrentStatus::Downloading);

        // Mock peer sends WRONG data (all zeros) for piece 0
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let wrong_data = vec![0u8; 16384];

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

            write_message(
                &mut w,
                &PeerMessage::Bitfield {
                    bytes: vec![0x80].into(),
                },
            )
            .await
            .unwrap();
            write_message(&mut w, &PeerMessage::Unchoke).await.unwrap();

            // Wait for Request, respond with corrupt data
            loop {
                match read_message(&mut r, DEFAULT_MAX_PEER_FRAME_LEN).await {
                    Ok(PeerMessage::Request {
                        index: 0,
                        begin: 0,
                        length: 16384,
                    }) => {
                        write_message(
                            &mut w,
                            &PeerMessage::Piece {
                                index: 0,
                                begin: 0,
                                block: wrong_data.into(),
                            },
                        )
                        .await
                        .unwrap();
                        break;
                    }
                    Ok(PeerMessage::Interested) => continue,
                    _ => break,
                }
            }
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });

        // Connect peer
        task.sources
            .add_candidate(SourceEndpoint::Peer(addr), SourceKind::Peer)
            .unwrap();
        let _ = task.discover_and_connect_peers().await.unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Tick: process Bitfield + Unchoke, request block
        let _ = task.tick().unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;

        // Tick: receive corrupt Piece, accept block
        let _ = task.tick().unwrap();

        // Verify should fail (HashMismatch) and quarantine the corrupt source.
        let events = task.verify_completed_pieces().await.unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, RuntimeEvent::SourceQuarantined { .. })),
            "expected quarantine event for corrupt piece; got: {events:?}"
        );
    }

    #[tokio::test]
    async fn t7_t3_corrupt_peer_piece_quarantines_source_during_verify() {
        let plan = make_verifiable_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();
        let info_hash = task.plan.info_hash;

        task.apply(TorrentCommand::Start).unwrap();
        task.tick().unwrap();
        assert_eq!(task.status, TorrentStatus::Downloading);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let wrong_data = vec![0u8; 16384];

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut r, mut w) = stream.into_split();
            let _handshake = read_handshake(&mut r, info_hash).await.unwrap();
            let response = Handshake {
                reserved: ExtensionBits::default(),
                info_hash,
                peer_id: PeerId::new([0xFE; 20]),
            };
            write_handshake(&mut w, &response).await.unwrap();
            write_message(
                &mut w,
                &PeerMessage::Bitfield {
                    bytes: vec![0x80].into(),
                },
            )
            .await
            .unwrap();
            write_message(&mut w, &PeerMessage::Unchoke).await.unwrap();

            loop {
                match read_message(&mut r, DEFAULT_MAX_PEER_FRAME_LEN).await {
                    Ok(PeerMessage::Request {
                        index: 0,
                        begin: 0,
                        length: 16384,
                    }) => {
                        write_message(
                            &mut w,
                            &PeerMessage::Piece {
                                index: 0,
                                begin: 0,
                                block: wrong_data.into(),
                            },
                        )
                        .await
                        .unwrap();
                        break;
                    }
                    Ok(PeerMessage::Interested) => continue,
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }

            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });

        let source_id = task
            .sources
            .add_candidate(SourceEndpoint::Peer(addr), SourceKind::Peer)
            .unwrap();
        let events = task.discover_and_connect_peers().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(task.sources.state(source_id).unwrap(), SourceState::Active);

        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = task.tick_and_verify().await.unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;
        let events = task.tick_and_verify().await.unwrap();

        assert!(
            events.iter().any(|e| matches!(
                e,
                RuntimeEvent::SourceQuarantined { source, .. } if source == &addr.to_string()
            )),
            "expected SourceQuarantined for corrupt peer; got: {events:?}"
        );
        assert_eq!(
            task.sources.state(source_id).unwrap(),
            SourceState::Quarantined
        );
        assert_eq!(task.peers.connected_count(), 0);
    }

    #[tokio::test]
    async fn t6_t1_snapshot_reflects_connected_peers() {
        let plan = make_v1_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();
        let info_hash = task.plan.info_hash;

        // Before: snapshot has 0 peers
        let snap = task.snapshot();
        assert_eq!(snap.peers, 0);

        // Start 2 mock peer servers
        let mut addrs = Vec::new();
        for _ in 0..2 {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            addrs.push(addr);
            let ih = info_hash;
            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let (mut r, mut w) = stream.into_split();
                let _ = read_handshake(&mut r, ih).await;
                let response = Handshake {
                    reserved: ExtensionBits::default(),
                    info_hash: ih,
                    peer_id: PeerId::new([0xFD; 20]),
                };
                let _ = write_handshake(&mut w, &response).await;
            });
        }

        for addr in &addrs {
            task.sources
                .add_candidate(SourceEndpoint::Peer(*addr), SourceKind::Peer)
                .unwrap();
        }
        let _ = task.discover_and_connect_peers().await.unwrap();

        // After: snapshot has 2 peers
        let snap = task.snapshot();
        assert_eq!(snap.peers, 2);
    }

    #[tokio::test]
    async fn t6_t4_seed_count_reflects_full_bitfield_peers() {
        let plan = make_v1_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();
        let info_hash = task.plan.info_hash;

        // Transition to Downloading so tick() processes messages
        task.apply(TorrentCommand::Start).unwrap();
        task.tick().unwrap();
        assert_eq!(task.status, TorrentStatus::Downloading);

        // Start a seed peer that sends full bitfield immediately
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let ih = info_hash;
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut r, mut w) = stream.into_split();
            let _ = read_handshake(&mut r, ih).await;
            let response = Handshake {
                reserved: ExtensionBits::default(),
                info_hash: ih,
                peer_id: PeerId::new([0xFC; 20]),
            };
            write_handshake(&mut w, &response).await.unwrap();
            write_message(
                &mut w,
                &PeerMessage::Bitfield {
                    bytes: vec![0xC0].into(),
                },
            )
            .await
            .unwrap();
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });

        task.sources
            .add_candidate(SourceEndpoint::Peer(addr), SourceKind::Peer)
            .unwrap();
        let _ = task.discover_and_connect_peers().await.unwrap();

        // Let bitfield arrive, then tick drains and processes it
        tokio::time::sleep(Duration::from_millis(200)).await;

        let _events = task.tick().unwrap();

        let snap = task.snapshot();
        assert_eq!(snap.seeds, 1, "expected 1 seed, got {}", snap.seeds);
    }

    #[tokio::test]
    async fn t5_t1_tracker_announce_feeds_sourcetable_and_connects_peers() {
        let info_hash = InfoHashV1::new(TEST_INFO_HASH);

        // Start mock TCP peer servers that respond with handshake
        let mut peer_addrs = Vec::new();
        for _ in 0..2 {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            peer_addrs.push(addr);
            let ih = info_hash;
            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let (mut r, mut w) = stream.into_split();
                let _ = read_handshake(&mut r, ih).await;
                let response = Handshake {
                    reserved: ExtensionBits::default(),
                    info_hash: ih,
                    peer_id: PeerId::new([0xFE; 20]),
                };
                let _ = write_handshake(&mut w, &response).await;
            });
        }

        // Start minimal HTTP tracker server that returns peer_addrs
        let tracker_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tracker_addr = tracker_listener.local_addr().unwrap();
        let connect_peers = peer_addrs.clone();
        tokio::spawn(async move {
            let (mut stream, _) = tracker_listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).await;
            let mut dict = BTreeMap::new();
            dict.insert(b"complete".to_vec(), BencodeValue::Integer(2));
            dict.insert(b"incomplete".to_vec(), BencodeValue::Integer(0));
            dict.insert(b"interval".to_vec(), BencodeValue::Integer(600));
            let mut compact = Vec::with_capacity(connect_peers.len() * 6);
            for p in &connect_peers {
                if let SocketAddr::V4(v4) = p {
                    compact.extend_from_slice(&v4.ip().octets());
                    compact.extend_from_slice(&v4.port().to_be_bytes());
                }
            }
            dict.insert(b"peers".to_vec(), BencodeValue::Bytes(Bytes::from(compact)));
            let body = styx_proto::encode(&BencodeValue::Dict(dict));
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes()).await;
            let _ = stream.write_all(&body).await;
        });

        // Create plan pointing at our mock tracker
        let mut tracker_plan = make_v1_plan();
        tracker_plan.announce_urls =
            vec![format!("http://{}/announce", tracker_addr).parse().unwrap()];
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(tracker_plan, config).unwrap();

        // Call discover_and_connect_peers — tracker announce → SourceTable → TCP connect
        let events = task.discover_and_connect_peers().await.unwrap();

        // Should have 3 events: no SourceFailed, and 2 PeerConnected
        let source_fails: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, RuntimeEvent::SourceFailed { .. }))
            .collect();
        assert!(
            source_fails.is_empty(),
            "unexpected SourceFailed: {source_fails:?}"
        );

        assert_eq!(
            events.len(),
            2,
            "expected 2 PeerConnected events; got: {events:?}"
        );
        for event in &events {
            assert!(matches!(event, RuntimeEvent::PeerConnected { .. }));
        }
        assert_eq!(task.peers.connected_count(), 2);
        assert!(task.last_announce.is_some());
    }

    #[tokio::test]
    async fn t4_t4_tick_emits_peer_disconnected_on_peer_drop() {
        let plan = make_v1_plan();
        let config = make_config();
        let mut task = TorrentTask::new_with_peers(plan, config).unwrap();
        let info_hash = task.plan.info_hash;

        // Transition to Downloading
        task.apply(TorrentCommand::Start).unwrap();
        task.tick().unwrap();
        assert_eq!(task.status, TorrentStatus::Downloading);

        // Start mock peer that disconnects after handshake
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
            // Drop halves to close the connection
            drop(r);
            drop(w);
        });

        // Connect peer
        task.sources
            .add_candidate(SourceEndpoint::Peer(addr), SourceKind::Peer)
            .unwrap();
        let events = task.discover_and_connect_peers().await.unwrap();
        assert_eq!(events.len(), 1);

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Tick should process the disconnect and emit PeerDisconnected event
        let events = task.tick().unwrap();
        let disconnect_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, RuntimeEvent::PeerDisconnected { .. }))
            .collect();
        assert!(
            !disconnect_events.is_empty(),
            "expected at least one PeerDisconnected event; got: {events:?}"
        );
    }

    #[tokio::test]
    async fn seeding_task_sends_piece_for_valid_request() {
        let mut task = TorrentTask::new_with_peers(make_verifiable_plan(), make_config()).unwrap();
        let piece = PieceIndex::new(0);
        task.accept_piece_blocks(
            piece,
            vec![(
                BlockSpec::new(
                    piece,
                    BlockOffset::new(0),
                    BlockLength::new(16_384).unwrap(),
                    16_384,
                )
                .unwrap(),
                Bytes::from(vec![0x42u8; 16_384]),
            )],
        )
        .await
        .unwrap();
        task.verify_completed_pieces().await.unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let info_hash = task.plan.info_hash;
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_handshake(&mut stream, info_hash).await.unwrap();
            write_handshake(
                &mut stream,
                &Handshake {
                    reserved: ExtensionBits::default(),
                    info_hash,
                    peer_id: PeerId::new([9; 20]),
                },
            )
            .await
            .unwrap();
            read_message(&mut stream, DEFAULT_MAX_PEER_FRAME_LEN)
                .await
                .unwrap()
        });
        let key = task
            .peers
            .connect_peer(addr, info_hash, task.peer_id, Duration::from_secs(2))
            .await
            .unwrap();

        let events = task
            .execute_actions_with_uploads(vec![PeerAction::ServeBlock {
                peer: key,
                request: BlockRequest::new(
                    piece,
                    BlockOffset::new(0),
                    BlockLength::new(16_384).unwrap(),
                ),
            }])
            .await;

        let message = tokio::time::timeout(Duration::from_secs(2), peer)
            .await
            .unwrap()
            .unwrap();
        assert!(events.contains(&RuntimeEvent::BlockUploaded {
            torrent: task.plan.id,
            peer: addr,
            piece: 0,
            offset: 0,
            bytes: 16_384,
        }));
        assert_eq!(
            message,
            PeerMessage::Piece {
                index: 0,
                begin: 0,
                block: Bytes::from(vec![0x42u8; 16_384]),
            }
        );
    }

    #[tokio::test]
    async fn seeding_task_rejects_request_for_unverified_piece() {
        let mut task = TorrentTask::new_with_peers(make_verifiable_plan(), make_config()).unwrap();
        let piece = PieceIndex::new(0);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let info_hash = task.plan.info_hash;
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_handshake(&mut stream, info_hash).await.unwrap();
            write_handshake(
                &mut stream,
                &Handshake {
                    reserved: ExtensionBits::default(),
                    info_hash,
                    peer_id: PeerId::new([8; 20]),
                },
            )
            .await
            .unwrap();
            read_message(&mut stream, DEFAULT_MAX_PEER_FRAME_LEN).await
        });
        let key = task
            .peers
            .connect_peer(addr, info_hash, task.peer_id, Duration::from_secs(2))
            .await
            .unwrap();

        let events = task
            .execute_actions_with_uploads(vec![PeerAction::ServeBlock {
                peer: key,
                request: BlockRequest::new(
                    piece,
                    BlockOffset::new(0),
                    BlockLength::new(16).unwrap(),
                ),
            }])
            .await;

        assert!(events.contains(&RuntimeEvent::PeerDisconnected {
            torrent: task.plan.id,
            addr,
        }));
        let _ = peer.await;
    }

    #[tokio::test]
    async fn completed_torrent_transitions_to_seeding_when_verified() {
        let mut task = TorrentTask::new_with_peers(make_verifiable_plan(), make_config()).unwrap();

        let events = task
            .complete_from_piece_bytes(vec![Bytes::from(vec![0x42u8; 16_384])])
            .await
            .unwrap();

        assert!(events.iter().any(|event| {
            matches!(
                event,
                RuntimeEvent::StateChanged {
                    from: TorrentStatus::Complete,
                    to: TorrentStatus::Seeding,
                    ..
                }
            )
        }));
        assert_eq!(task.status, TorrentStatus::Seeding);
    }

    #[tokio::test]
    async fn seeding_tick_responds_to_interested_requesting_peer() {
        let mut task = TorrentTask::new_with_peers(make_verifiable_plan(), make_config()).unwrap();
        task.complete_from_piece_bytes(vec![Bytes::from(vec![0x42u8; 16_384])])
            .await
            .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let info_hash = task.plan.info_hash;
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_handshake(&mut stream, info_hash).await.unwrap();
            write_handshake(
                &mut stream,
                &Handshake {
                    reserved: ExtensionBits::default(),
                    info_hash,
                    peer_id: PeerId::new([7; 20]),
                },
            )
            .await
            .unwrap();
            write_message(&mut stream, &PeerMessage::Interested)
                .await
                .unwrap();
            let first = read_message(&mut stream, DEFAULT_MAX_PEER_FRAME_LEN)
                .await
                .unwrap();
            write_message(
                &mut stream,
                &PeerMessage::Request {
                    index: 0,
                    begin: 0,
                    length: 16_384,
                },
            )
            .await
            .unwrap();
            let second = read_message(&mut stream, DEFAULT_MAX_PEER_FRAME_LEN)
                .await
                .unwrap();
            (first, second)
        });
        let key = task
            .peers
            .connect_peer(addr, info_hash, task.peer_id, Duration::from_secs(2))
            .await
            .unwrap();
        task.manager.add_peer(key).unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = task.tick_seed_and_upload().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let events = task.tick_seed_and_upload().await.unwrap();

        assert!(events.iter().any(|event| {
            matches!(
                event,
                RuntimeEvent::BlockUploaded {
                    piece: 0,
                    bytes: 16_384,
                    ..
                }
            )
        }));
        assert_eq!(task.snapshot().uploaded_bytes, 16_384);
        let (first, second) = tokio::time::timeout(Duration::from_secs(2), peer)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first, PeerMessage::Unchoke);
        assert_eq!(
            second,
            PeerMessage::Piece {
                index: 0,
                begin: 0,
                block: Bytes::from(vec![0x42u8; 16_384]),
            }
        );
    }
}
