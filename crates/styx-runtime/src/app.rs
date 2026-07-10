use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use styx_app::{
    commands::{CommandResponse, ControlCommand},
    error::AppError,
    events::AppEvent,
    format::InfoHashHex,
    model::{
        AppSnapshot, LogLevel, LogLine, SessionTotals, SpeedSample, TorrentRow,
        TorrentStatus as AppStatus,
    },
    TorrentRuntime,
};

use crate::{
    driver::{spawn_bg_download, spawn_bg_magnet_resolution, spawn_bg_seed, BgEvent},
    MagnetAdd, PersistentState, PersistentStore, PersistentTorrent, PersistentTorrentSource,
    PersistentTorrentState, RuntimeCommand, RuntimeConfig, RuntimeEngine, RuntimeError,
    RuntimeEvent, RuntimeSnapshot, TorrentCommand, TorrentId, TorrentPlan, TorrentSnapshot,
    TorrentStatus, PERSISTENT_STATE_SCHEMA_VERSION,
};

const DEFAULT_SPEED_SAMPLES: usize = 60;
const MAX_LOG_LINES: usize = 1000;
const DHT_ANNOUNCE_INTERVAL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug)]
pub struct AppRuntime {
    engine: RuntimeEngine,
    speed: VecDeque<SpeedSample>,
    logs: VecDeque<LogLine>,
    tick_count: u64,
    bg_tx: mpsc::UnboundedSender<BgEvent>,
    bg_rx: mpsc::UnboundedReceiver<BgEvent>,
    bg_handles: HashMap<TorrentId, tokio::task::JoinHandle<()>>,
    pending_plans: HashMap<TorrentId, TorrentPlan>,
    pending_magnets: HashMap<TorrentId, MagnetAdd>,
    pending_download_peers: HashMap<TorrentId, Vec<std::net::SocketAddr>>,
    persistent_torrents: BTreeMap<TorrentId, PersistentTorrent>,
    dht_worker: Option<crate::DhtWorkerHandle>,
    dht_events: Option<mpsc::UnboundedReceiver<crate::DhtRuntimeEvent>>,
    dht_bootstrapped: bool,
    dht_announce_ready: HashSet<TorrentId>,
    dht_last_announce: HashMap<TorrentId, Instant>,
    lsd_worker: Option<crate::LsdWorkerHandle>,
    lsd_events: Option<mpsc::UnboundedReceiver<crate::LsdRuntimeEvent>>,
    lsd_targets: Vec<(TorrentId, styx_proto::InfoHashV1)>,
}

#[derive(Debug)]
pub struct PersistentAppRuntime {
    runtime: AppRuntime,
    store: PersistentStore,
}

impl AppRuntime {
    pub fn new(engine: RuntimeEngine) -> Self {
        let (bg_tx, bg_rx) = mpsc::unbounded_channel();
        Self {
            engine,
            speed: VecDeque::with_capacity(DEFAULT_SPEED_SAMPLES),
            logs: VecDeque::with_capacity(MAX_LOG_LINES),
            tick_count: 0,
            bg_tx,
            bg_rx,
            bg_handles: HashMap::new(),
            pending_plans: HashMap::new(),
            pending_magnets: HashMap::new(),
            pending_download_peers: HashMap::new(),
            persistent_torrents: BTreeMap::new(),
            dht_worker: None,
            dht_events: None,
            dht_bootstrapped: false,
            dht_announce_ready: HashSet::new(),
            dht_last_announce: HashMap::new(),
            lsd_worker: None,
            lsd_events: None,
            lsd_targets: Vec::new(),
        }
    }

    pub fn into_engine(self) -> RuntimeEngine {
        self.engine
    }

    pub fn new_with_config(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        Ok(Self::new(RuntimeEngine::new(config)?))
    }

    pub async fn restore_from_state(
        config: RuntimeConfig,
        state: PersistentState,
    ) -> Result<Self, RuntimeError> {
        let state = state.validate()?;
        let mut runtime = Self::new_with_config(config)?;
        for torrent in state.torrents {
            let PersistentTorrentSource::File { path: source_path } = &torrent.source else {
                let uri = match &torrent.source {
                    PersistentTorrentSource::Magnet { uri } => uri.clone(),
                    PersistentTorrentSource::File { .. } => unreachable!(),
                };
                let magnet = styx_proto::parse_magnet_uri(&uri)
                    .map_err(|_| RuntimeError::Persistence("invalid persistent magnet uri"))?;
                let info_hash = magnet.info_hash_v1.ok_or(RuntimeError::Persistence(
                    "persistent magnet requires a v1 info hash",
                ))?;
                let id = TorrentId::new(info_hash);
                runtime.pending_magnets.insert(
                    id,
                    MagnetAdd {
                        uri,
                        destination: torrent.destination.clone(),
                    },
                );
                runtime.persistent_torrents.insert(id, torrent);
                continue;
            };
            if !source_path.exists() {
                return Err(RuntimeError::Persistence(
                    "persistent torrent source is missing",
                ));
            }
            let plan = TorrentPlan::from_file(source_path, &torrent.destination)?;
            let id = plan.id;
            runtime
                .engine
                .apply(RuntimeCommand::AddPlan(Box::new(plan.clone())))?;
            match torrent.state {
                PersistentTorrentState::Paused => {
                    runtime
                        .engine
                        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))?;
                    runtime
                        .engine
                        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Pause))?;
                }
                PersistentTorrentState::Complete => {
                    let summary = runtime.engine.resume_verify(id).await?;
                    if summary.verified == plan.piece_count()
                        && summary.missing == 0
                        && summary.failed == 0
                    {
                        for event in runtime.engine.replace_with_completed(id)? {
                            runtime.engine.push_event(event);
                        }
                        if runtime.engine.config().seed_policy.seed_after_complete {
                            runtime.spawn_seed_worker(id, plan.clone());
                        }
                    } else {
                        runtime
                            .engine
                            .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))?;
                        runtime
                            .engine
                            .apply(RuntimeCommand::Torrent(id, TorrentCommand::Pause))?;
                    }
                }
                PersistentTorrentState::Queued
                | PersistentTorrentState::Downloading
                | PersistentTorrentState::Failed => {
                    let _ = runtime.engine.resume_verify(id).await?;
                    runtime.pending_plans.insert(id, plan);
                    runtime
                        .engine
                        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))?;
                }
            }
            runtime.persistent_torrents.insert(id, torrent);
        }
        Ok(runtime)
    }

    #[must_use]
    pub fn persistent_state(&mut self) -> PersistentState {
        PersistentState {
            schema_version: PERSISTENT_STATE_SCHEMA_VERSION,
            torrents: self.persistent_torrents.values().cloned().collect(),
        }
    }

    pub fn attach_dht_worker(
        &mut self,
        worker: crate::DhtWorkerHandle,
        events: mpsc::UnboundedReceiver<crate::DhtRuntimeEvent>,
    ) -> Result<(), RuntimeError> {
        worker.send(crate::DhtCommand::Bootstrap)?;
        self.dht_worker = Some(worker);
        self.dht_events = Some(events);
        Ok(())
    }

    pub fn attach_lsd_worker(
        &mut self,
        worker: crate::LsdWorkerHandle,
        events: mpsc::UnboundedReceiver<crate::LsdRuntimeEvent>,
    ) {
        self.lsd_worker = Some(worker);
        self.lsd_events = Some(events);
    }

    fn request_dht_peers(&self, id: TorrentId, add: &MagnetAdd) -> Result<(), RuntimeError> {
        let magnet = styx_proto::parse_magnet_uri(&add.uri)
            .map_err(|err| RuntimeError::Magnet(err.to_string()))?;
        let info_hash = magnet
            .info_hash_v1
            .ok_or_else(|| RuntimeError::Magnet("v1 info hash required for DHT lookup".into()))?;
        self.request_dht_lookup(id, styx_dht::InfoHash::new(*info_hash.as_bytes()))
    }

    fn request_dht_lookup(
        &self,
        id: TorrentId,
        info_hash: styx_dht::InfoHash,
    ) -> Result<(), RuntimeError> {
        let Some(worker) = &self.dht_worker else {
            return Ok(());
        };
        if !self.dht_bootstrapped {
            return Ok(());
        }
        worker.send(crate::DhtCommand::GetPeers {
            torrent: id,
            info_hash,
        })
    }

    fn announce_dht_if_ready(&mut self, id: TorrentId) {
        if !self.dht_announce_ready.contains(&id)
            || self.engine.config().listen_port == 0
            || self
                .dht_last_announce
                .get(&id)
                .is_some_and(|last| last.elapsed() < DHT_ANNOUNCE_INTERVAL)
        {
            return;
        }
        let Some(info_hash) = self.engine.dht_announce_target(id) else {
            return;
        };
        let Some(worker) = &self.dht_worker else {
            return;
        };
        if worker
            .send(crate::DhtCommand::AnnouncePeer {
                torrent: id,
                info_hash,
                port: self.engine.config().listen_port,
                implied_port: false,
            })
            .is_ok()
        {
            self.dht_last_announce.insert(id, Instant::now());
        }
    }

    fn apply_add(
        &mut self,
        source: std::path::PathBuf,
        destination: Option<std::path::PathBuf>,
    ) -> Result<CommandResponse, AppError> {
        let dest =
            destination.ok_or_else(|| AppError::InvalidCommand("destination required".into()))?;
        let plan = TorrentPlan::from_file(&source, &dest).map_err(|e| match e {
            RuntimeError::Io(io_err) => AppError::ReadTorrent {
                path: source.clone(),
                source: io_err,
            },
            RuntimeError::Torrent(parse_err) => AppError::ParseTorrent {
                path: source.clone(),
                source: parse_err,
            },
            other => AppError::Internal(other.to_string()),
        })?;
        let id = plan.id;
        let name = plan.name.clone();
        let info_hash = InfoHashHex::new(*id.as_bytes());
        self.persistent_torrents.insert(
            id,
            PersistentTorrent {
                source: PersistentTorrentSource::File {
                    path: source.clone(),
                },
                destination: dest.clone(),
                state: PersistentTorrentState::Downloading,
                added_at_unix: 0,
                completed_at_unix: None,
            },
        );
        self.pending_plans.insert(id, plan.clone());
        self.engine
            .apply(RuntimeCommand::AddPlan(Box::new(plan)))
            .map_err(map_runtime_error)?;
        self.engine
            .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))
            .map_err(map_runtime_error)?;
        if let Some(info_hash) = self.engine.dht_announce_target(id) {
            self.request_dht_lookup(id, info_hash)
                .map_err(map_runtime_error)?;
        }
        Ok(CommandResponse::TorrentAdded { info_hash, name })
    }

    fn apply_add_magnet(
        &mut self,
        uri: String,
        destination: Option<std::path::PathBuf>,
    ) -> Result<CommandResponse, AppError> {
        let destination =
            destination.ok_or_else(|| AppError::InvalidCommand("destination required".into()))?;
        let add = MagnetAdd {
            uri: uri.clone(),
            destination: destination.clone(),
        };
        self.engine
            .apply(RuntimeCommand::AddMagnet(Box::new(add.clone())))
            .map_err(map_runtime_error)?;
        let magnet = styx_proto::parse_magnet_uri(&uri)
            .map_err(|err| AppError::InvalidCommand(err.to_string()))?;
        let info_hash = magnet.info_hash_v1.ok_or_else(|| {
            AppError::InvalidCommand(
                "v1 info hash required until v2 downloads are supported".into(),
            )
        })?;
        let id = TorrentId::new(info_hash);
        if self.persistent_torrents.contains_key(&id) {
            return Err(AppError::InvalidCommand("torrent already exists".into()));
        }
        let name = magnet.display_name.unwrap_or_default();
        self.persistent_torrents.insert(
            id,
            PersistentTorrent {
                source: PersistentTorrentSource::Magnet { uri },
                destination,
                state: PersistentTorrentState::Queued,
                added_at_unix: 0,
                completed_at_unix: None,
            },
        );
        self.pending_magnets.insert(id, add.clone());
        if magnet.exact_peers.is_empty() {
            self.request_dht_peers(id, &add)
                .map_err(map_runtime_error)?;
        } else if let Some(handle) = spawn_bg_magnet_resolution(
            id,
            add,
            self.bg_tx.clone(),
            self.engine.config().clone(),
            Vec::new(),
        ) {
            self.bg_handles.insert(id, handle);
        }
        Ok(CommandResponse::TorrentAdded {
            info_hash: InfoHashHex::new(*id.as_bytes()),
            name,
        })
    }

    fn spawn_seed_worker(&mut self, id: TorrentId, plan: TorrentPlan) {
        if self.bg_handles.contains_key(&id) {
            return;
        }
        let tx = self.bg_tx.clone();
        let config = self.engine.config().clone();
        if let Some(handle) = spawn_bg_seed(plan, tx, config) {
            self.bg_handles.insert(id, handle);
        }
    }
}

impl PersistentAppRuntime {
    pub async fn open(config: RuntimeConfig, store: PersistentStore) -> Result<Self, RuntimeError> {
        let state = store.load()?;
        let runtime = AppRuntime::restore_from_state(config, state).await?;
        Ok(Self { runtime, store })
    }

    pub fn apply_and_persist(
        &mut self,
        command: ControlCommand,
    ) -> Result<CommandResponse, AppError> {
        let should_persist = !matches!(command, ControlCommand::Status);
        let response = self.runtime.apply(command)?;
        if should_persist {
            let state = self.runtime.persistent_state();
            self.store
                .save(&state)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        Ok(response)
    }

    pub fn tick_and_persist(&mut self) -> Result<Vec<AppEvent>, AppError> {
        let before = self.runtime.persistent_state();
        let events = self.runtime.tick();
        let after = self.runtime.persistent_state();
        if after != before {
            self.store
                .save(&after)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        Ok(events)
    }

    pub fn persist_now(&mut self) -> Result<(), RuntimeError> {
        let state = self.runtime.persistent_state();
        self.store.save(&state)
    }

    #[must_use]
    pub fn runtime_mut(&mut self) -> &mut AppRuntime {
        &mut self.runtime
    }
}

impl TorrentRuntime for AppRuntime {
    fn apply(&mut self, command: ControlCommand) -> Result<CommandResponse, AppError> {
        match command {
            ControlCommand::Add {
                source,
                destination,
            } => self.apply_add(source, destination),
            ControlCommand::AddMagnet { uri, destination } => {
                self.apply_add_magnet(uri, destination)
            }
            ControlCommand::Remove { info_hash } => {
                let id = torrent_id_from_hex(info_hash)?;
                if let Some(handle) = self.bg_handles.remove(&id) {
                    handle.abort();
                }
                self.pending_plans.remove(&id);
                self.persistent_torrents.remove(&id);
                self.engine
                    .apply(RuntimeCommand::Remove(id))
                    .map_err(map_runtime_error)?;
                let info_hash_hex = InfoHashHex::new(*id.as_bytes());
                Ok(CommandResponse::TorrentRemoved {
                    info_hash: info_hash_hex,
                })
            }
            ControlCommand::Pause { info_hash } => {
                let id = torrent_id_from_hex(info_hash)?;
                let was_seeding =
                    self.engine.snapshot().torrents.iter().any(|snapshot| {
                        snapshot.id == id && snapshot.status == TorrentStatus::Seeding
                    });
                self.engine
                    .apply(RuntimeCommand::Torrent(id, TorrentCommand::Pause))
                    .map_err(map_runtime_error)?;
                if was_seeding {
                    if let Some(handle) = self.bg_handles.remove(&id) {
                        handle.abort();
                    }
                }
                if let Some(torrent) = self.persistent_torrents.get_mut(&id) {
                    torrent.state = PersistentTorrentState::Paused;
                }
                Ok(CommandResponse::TorrentPaused {
                    info_hash: InfoHashHex::new(*id.as_bytes()),
                })
            }
            ControlCommand::Resume { info_hash } => {
                let id = torrent_id_from_hex(info_hash)?;
                self.engine
                    .apply(RuntimeCommand::Torrent(id, TorrentCommand::Resume))
                    .map_err(map_runtime_error)?;
                if let Some(torrent) = self.persistent_torrents.get_mut(&id) {
                    let PersistentTorrentSource::File { path } = &torrent.source else {
                        return Err(AppError::InvalidCommand(
                            "magnet metadata is not resolved".into(),
                        ));
                    };
                    let plan = TorrentPlan::from_file(path, &torrent.destination)
                        .map_err(map_runtime_error)?;
                    if self.engine.snapshot().torrents.iter().any(|snapshot| {
                        snapshot.id == id && snapshot.status == TorrentStatus::Seeding
                    }) {
                        torrent.state = PersistentTorrentState::Complete;
                        self.spawn_seed_worker(id, plan);
                    } else {
                        torrent.state = PersistentTorrentState::Downloading;
                        self.pending_plans.insert(id, plan);
                    }
                }
                Ok(CommandResponse::TorrentResumed {
                    info_hash: InfoHashHex::new(*id.as_bytes()),
                })
            }
            ControlCommand::Status => Ok(CommandResponse::Status {
                snapshot: self.snapshot(),
            }),
        }
    }

    fn snapshot(&mut self) -> AppSnapshot {
        let snap = self.engine.snapshot();
        let torrents: Vec<TorrentRow> = snap.torrents.iter().map(torrent_snapshot_to_row).collect();
        let totals = SessionTotals {
            torrent_count: torrents.len() as u32,
            peer_count: snap.peers.len() as u32,
            down_bytes: torrents.iter().map(|t| t.down_rate).sum(),
            up_bytes: torrents.iter().map(|t| t.up_rate).sum(),
        };
        AppSnapshot {
            torrents,
            peers: Vec::new(),
            speed: self.speed.iter().cloned().collect(),
            logs: self.logs.iter().cloned().collect(),
            totals,
        }
    }

    fn tick(&mut self) -> Vec<AppEvent> {
        let mut dht_events = Vec::new();
        if let Some(events) = &mut self.dht_events {
            while let Ok(event) = events.try_recv() {
                dht_events.push(event);
            }
        }
        for event in dht_events {
            match event {
                crate::DhtRuntimeEvent::Bootstrapped { .. } => {
                    self.dht_bootstrapped = true;
                    for (id, add) in self.pending_magnets.clone() {
                        let _ = self.request_dht_peers(id, &add);
                    }
                    for (id, info_hash) in self.engine.dht_announce_targets() {
                        let _ = self.request_dht_lookup(id, info_hash);
                    }
                }
                crate::DhtRuntimeEvent::PeersDiscovered { torrent, peers } => {
                    self.dht_announce_ready.insert(torrent);
                    if let Some(add) = self.pending_magnets.get(&torrent).cloned() {
                        self.pending_download_peers.insert(torrent, peers.clone());
                        if let Some(handle) = spawn_bg_magnet_resolution(
                            torrent,
                            add,
                            self.bg_tx.clone(),
                            self.engine.config().clone(),
                            peers,
                        ) {
                            self.bg_handles.insert(torrent, handle);
                        }
                    } else {
                        let _ = self.engine.add_dht_peers(torrent, peers);
                        self.announce_dht_if_ready(torrent);
                    }
                }
                crate::DhtRuntimeEvent::LookupExhausted { torrent } => {
                    self.dht_announce_ready.insert(torrent);
                    if self.pending_magnets.contains_key(&torrent) {
                        if let Some(record) = self.persistent_torrents.get_mut(&torrent) {
                            record.state = PersistentTorrentState::Failed;
                        }
                        self.engine.push_event(RuntimeEvent::TaskFailed {
                            torrent,
                            reason: "DHT peer lookup exhausted".to_owned(),
                        });
                    } else {
                        self.announce_dht_if_ready(torrent);
                    }
                }
                crate::DhtRuntimeEvent::Announced { torrent, nodes } => {
                    self.engine
                        .push_event(RuntimeEvent::DhtAnnounced { torrent, nodes });
                }
                _ => {}
            }
        }
        for (id, _) in self.engine.dht_announce_targets() {
            self.announce_dht_if_ready(id);
        }
        if let Some(events) = &mut self.lsd_events {
            while let Ok(crate::LsdRuntimeEvent::PeerDiscovered { torrent, peer }) =
                events.try_recv()
            {
                let _ = self.engine.add_lsd_peer(torrent, peer);
            }
        }
        let lsd_targets = self.engine.lsd_announce_targets();
        if lsd_targets != self.lsd_targets {
            self.lsd_targets = lsd_targets.clone();
            if let Some(worker) = &self.lsd_worker {
                let _ = worker.send(crate::LsdCommand::Update {
                    torrents: lsd_targets,
                });
            }
        }
        // 1. Process background download events
        while let Ok(bg) = self.bg_rx.try_recv() {
            match bg {
                BgEvent::Progress {
                    id,
                    verified_bytes,
                    total_bytes: _,
                } => {
                    let _ = self.engine.sync_progress(id, verified_bytes);
                }
                BgEvent::Completed { id } => {
                    self.bg_handles.remove(&id);
                    let plan = self.pending_plans.remove(&id);
                    if let Some(torrent) = self.persistent_torrents.get_mut(&id) {
                        torrent.state = PersistentTorrentState::Complete;
                        torrent.completed_at_unix = Some(0);
                    }
                    if let Ok(events) = self.engine.replace_with_completed(id) {
                        for e in events {
                            self.engine.push_event(e);
                        }
                    }
                    if let Some(plan) =
                        plan.filter(|_| self.engine.config().seed_policy.seed_after_complete)
                    {
                        self.spawn_seed_worker(id, plan);
                    }
                }
                BgEvent::Failed { id, reason } => {
                    self.bg_handles.remove(&id);
                    self.pending_plans.remove(&id);
                    if let Some(torrent) = self.persistent_torrents.get_mut(&id) {
                        torrent.state = PersistentTorrentState::Failed;
                    }
                    self.engine.push_event(RuntimeEvent::TaskFailed {
                        torrent: id,
                        reason,
                    });
                }
                BgEvent::SourceFailed { id, source, reason } => {
                    self.engine.push_event(RuntimeEvent::SourceFailed {
                        torrent: id,
                        source,
                        reason,
                    });
                }
                BgEvent::PeerDisconnected { id, addr } => {
                    self.engine
                        .push_event(RuntimeEvent::PeerDisconnected { torrent: id, addr });
                }
                BgEvent::Runtime { event } => {
                    self.engine.push_event(event);
                }
                BgEvent::MagnetMetadataResolved { id, plan, peers } => {
                    self.bg_handles.remove(&id);
                    self.pending_magnets.remove(&id);
                    let peers = if plan.is_private() { Vec::new() } else { peers };
                    self.pending_download_peers.insert(id, peers);
                    let plan = *plan;
                    if let Err(err) = self
                        .engine
                        .apply(RuntimeCommand::AddPlan(Box::new(plan.clone())))
                        .and_then(|_| {
                            self.engine
                                .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))
                        })
                    {
                        self.engine.push_event(RuntimeEvent::TaskFailed {
                            torrent: id,
                            reason: err.to_string(),
                        });
                        if let Some(torrent) = self.persistent_torrents.get_mut(&id) {
                            torrent.state = PersistentTorrentState::Failed;
                        }
                        continue;
                    }
                    self.pending_plans.insert(id, plan);
                    if let Some(torrent) = self.persistent_torrents.get_mut(&id) {
                        torrent.state = PersistentTorrentState::Downloading;
                    }
                    self.announce_dht_if_ready(id);
                }
                BgEvent::MagnetResolutionFailed {
                    id,
                    reason,
                    try_dht,
                } => {
                    self.bg_handles.remove(&id);
                    if try_dht && self.dht_worker.is_some() {
                        if let Some(add) = self.pending_magnets.get(&id) {
                            let _ = self.request_dht_peers(id, add);
                            continue;
                        }
                    }
                    if let Some(torrent) = self.persistent_torrents.get_mut(&id) {
                        torrent.state = PersistentTorrentState::Failed;
                    }
                    self.engine.push_event(RuntimeEvent::TaskFailed {
                        torrent: id,
                        reason,
                    });
                }
            }
        }

        // 2. Spawn bg tasks for torrents in Discovering state
        {
            let snap = self.engine.snapshot();
            for tor in &snap.torrents {
                if tor.status == TorrentStatus::Discovering
                    && !self.bg_handles.contains_key(&tor.id)
                {
                    if let Some(plan) = self.pending_plans.get(&tor.id).cloned() {
                        let tx = self.bg_tx.clone();
                        let config = self.engine.config().clone();
                        let peers = self
                            .pending_download_peers
                            .remove(&tor.id)
                            .unwrap_or_default();
                        if let Some(handle) = spawn_bg_download(plan, tx, config, peers) {
                            self.bg_handles.insert(tor.id, handle);
                        }
                    }
                }
            }
        }

        // 3. Drain engine events
        let engine_events = self.engine.drain_events();
        let snap = self.engine.snapshot();

        let mut app_events = Vec::new();
        let mut progress_changed = false;

        for event in &engine_events {
            if matches!(
                event,
                RuntimeEvent::ProgressUpdated { .. } | RuntimeEvent::TaskCompleted { .. }
            ) {
                progress_changed = true;
            }
            if let Some(app_event) = map_to_app_event(event, &snap) {
                app_events.push(app_event);
            }
            if let Some(log) = map_to_log_line(event) {
                if self.logs.len() >= MAX_LOG_LINES {
                    self.logs.pop_front();
                }
                self.logs.push_back(log);
            }
        }

        // 4. Emit snapshot event on progress change for GUI push
        if progress_changed {
            app_events.push(AppEvent::Snapshot {
                snapshot: self.snapshot(),
            });
        }

        // 5. Speed samples
        let total_down: u64 = snap.torrents.iter().map(|t| t.down_rate).sum();
        let total_up: u64 = snap.torrents.iter().map(|t| t.up_rate).sum();
        if self.speed.len() >= DEFAULT_SPEED_SAMPLES {
            self.speed.pop_front();
        }
        self.speed.push_back(SpeedSample {
            second: self.tick_count,
            down_rate: total_down,
            up_rate: total_up,
        });
        self.tick_count += 1;

        app_events
    }
}

fn torrent_id_from_hex(hex: InfoHashHex) -> Result<TorrentId, AppError> {
    use styx_proto::InfoHashV1;
    let bytes = *hex.as_bytes();
    Ok(TorrentId::new(InfoHashV1::new(bytes)))
}

fn map_runtime_error(err: RuntimeError) -> AppError {
    match err {
        RuntimeError::InvalidConfig(msg) => AppError::InvalidCommand(msg.to_string()),
        RuntimeError::Magnet(message) => AppError::InvalidCommand(message),
        RuntimeError::Backpressure { .. } => AppError::Backpressure,
        _ => AppError::Internal(err.to_string()),
    }
}

fn map_to_app_event(event: &RuntimeEvent, snap: &RuntimeSnapshot) -> Option<AppEvent> {
    match event {
        RuntimeEvent::TorrentAdded { torrent } => {
            let info_hash = InfoHashHex::new(*torrent.as_bytes());
            let name = snap
                .torrents
                .iter()
                .find(|t| t.id == *torrent)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            Some(AppEvent::TorrentAdded { info_hash, name })
        }
        RuntimeEvent::TorrentRemoved { torrent } => Some(AppEvent::TorrentRemoved {
            info_hash: InfoHashHex::new(*torrent.as_bytes()),
        }),
        RuntimeEvent::TaskCompleted { torrent } => {
            let info_hash = InfoHashHex::new(*torrent.as_bytes());
            let name = snap
                .torrents
                .iter()
                .find(|t| t.id == *torrent)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            Some(AppEvent::TorrentCompleted { info_hash, name })
        }
        _ => None,
    }
}

fn map_to_log_line(event: &RuntimeEvent) -> Option<LogLine> {
    match event {
        RuntimeEvent::TorrentAdded { torrent } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("torrent {:?} added", torrent),
        }),
        RuntimeEvent::TorrentRemoved { torrent } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("torrent {:?} removed", torrent),
        }),
        RuntimeEvent::StateChanged { torrent, from, to } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("torrent {torrent:?}: {from:?} → {to:?}"),
        }),
        RuntimeEvent::SourceFailed {
            torrent, source, ..
        } => Some(LogLine {
            level: LogLevel::Warn,
            message: format!("source {source} failed for torrent {torrent:?}"),
        }),
        RuntimeEvent::TaskFailed { torrent, reason } => Some(LogLine {
            level: LogLevel::Error,
            message: format!("torrent {torrent:?} failed: {reason}"),
        }),
        RuntimeEvent::PieceVerified {
            torrent,
            piece,
            bytes,
        } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("torrent {torrent:?} piece {piece} verified ({bytes} bytes)"),
        }),
        RuntimeEvent::PeerConnected { torrent, addr } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("peer {addr} connected for torrent {torrent:?}"),
        }),
        RuntimeEvent::PeerDisconnected { torrent, addr } => Some(LogLine {
            level: LogLevel::Warn,
            message: format!("peer {addr} disconnected for torrent {torrent:?}"),
        }),
        RuntimeEvent::DhtPeersDiscovered { torrent, peers } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("DHT discovered {peers} peers for torrent {torrent:?}"),
        }),
        RuntimeEvent::DhtAnnounced { torrent, nodes } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("announced torrent {torrent:?} to {nodes} DHT nodes"),
        }),
        RuntimeEvent::BlockUploaded {
            torrent,
            peer,
            piece,
            bytes,
            ..
        } => Some(LogLine {
            level: LogLevel::Info,
            message: format!(
                "uploaded {bytes} bytes from piece {piece} to peer {peer} for torrent {torrent:?}"
            ),
        }),
        RuntimeEvent::TaskCompleted { torrent } => Some(LogLine {
            level: LogLevel::Info,
            message: format!("torrent {torrent:?} completed"),
        }),
        _ => None,
    }
}

fn torrent_snapshot_to_row(snap: &TorrentSnapshot) -> TorrentRow {
    let progress = snap.progress();
    TorrentRow {
        info_hash: InfoHashHex::new(*snap.id.as_bytes()),
        name: snap.name.clone(),
        status: match snap.status {
            TorrentStatus::Checking | TorrentStatus::Discovering => AppStatus::Checking,
            TorrentStatus::Downloading => AppStatus::Downloading,
            TorrentStatus::Paused => AppStatus::Paused,
            TorrentStatus::Complete | TorrentStatus::Seeding => AppStatus::Seeding,
            TorrentStatus::Failed | TorrentStatus::Cancelled => AppStatus::Error,
        },
        size_bytes: snap.total_bytes,
        progress,
        uploaded_bytes: snap.uploaded_bytes,
        share_ratio: snap.share_ratio(),
        down_rate: snap.down_rate,
        up_rate: snap.up_rate,
        peers: snap.peers,
        seeds: snap.seeds,
    }
}
