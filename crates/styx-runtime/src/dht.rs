use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use styx_dht::{
    DhtConfig, DhtError, DhtEvent, DhtMessage, DhtRuntime, DhtSocket, InfoHash, NodeAddr, NodeId,
    TokenManager, TransactionKind,
};
use tokio::sync::{mpsc, Mutex};

use crate::{RuntimeError, TorrentId};

const DEFAULT_METADATA_SIZE_LIMIT: u64 = 8 * 1024 * 1024;
const DEFAULT_METADATA_REQUEST_LIMIT: u32 = 512;
const DEFAULT_DHT_TICK_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DhtRuntimeConfig {
    pub enabled: bool,
    pub bind: SocketAddr,
    pub bootstrap_nodes: Vec<SocketAddr>,
    pub query_timeout: Duration,
    pub metadata_size_limit: u64,
    pub metadata_request_limit: u32,
    pub tick_interval: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DhtCommand {
    Bootstrap,
    GetPeers {
        torrent: TorrentId,
        info_hash: InfoHash,
    },
    AnnouncePeer {
        torrent: TorrentId,
        info_hash: InfoHash,
        port: u16,
        implied_port: bool,
    },
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DhtRuntimeEvent {
    Bootstrapped {
        nodes: usize,
    },
    PeersDiscovered {
        torrent: TorrentId,
        peers: Vec<SocketAddr>,
    },
    LookupExhausted {
        torrent: TorrentId,
    },
    Announced {
        torrent: TorrentId,
        nodes: u32,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug)]
pub struct DhtWorkerHandle {
    tx: mpsc::UnboundedSender<DhtCommand>,
    join: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl Default for DhtRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 6881),
            bootstrap_nodes: Vec::new(),
            query_timeout: Duration::from_secs(15),
            metadata_size_limit: DEFAULT_METADATA_SIZE_LIMIT,
            metadata_request_limit: DEFAULT_METADATA_REQUEST_LIMIT,
            tick_interval: DEFAULT_DHT_TICK_INTERVAL,
        }
    }
}

impl DhtRuntimeConfig {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.bind.port() == 0 && !self.bind.ip().is_loopback() {
            return Err(RuntimeError::InvalidConfig(
                "dht bind port 0 is only allowed for loopback tests",
            ));
        }
        if self.query_timeout.is_zero() {
            return Err(RuntimeError::InvalidConfig(
                "dht query_timeout must be greater than zero",
            ));
        }
        if self.metadata_size_limit == 0 {
            return Err(RuntimeError::InvalidConfig(
                "dht metadata_size_limit must be greater than zero",
            ));
        }
        if self.metadata_request_limit == 0 {
            return Err(RuntimeError::InvalidConfig(
                "dht metadata_request_limit must be greater than zero",
            ));
        }
        if self.tick_interval.is_zero() {
            return Err(RuntimeError::InvalidConfig(
                "dht tick_interval must be greater than zero",
            ));
        }
        Ok(())
    }
}

impl DhtWorkerHandle {
    pub fn send(&self, command: DhtCommand) -> Result<(), RuntimeError> {
        self.tx.send(command).map_err(|_| RuntimeError::Cancelled)
    }

    pub async fn shutdown(self) -> Result<(), RuntimeError> {
        let _ = self.tx.send(DhtCommand::Shutdown);
        if let Some(join) = self.join.lock().await.take() {
            let _ = join.await;
        }
        Ok(())
    }
}

pub async fn spawn_dht_worker(
    config: DhtRuntimeConfig,
    events: mpsc::UnboundedSender<DhtRuntimeEvent>,
) -> Result<DhtWorkerHandle, RuntimeError> {
    config.validate()?;
    if !config.enabled {
        return Err(RuntimeError::InvalidConfig("dht worker is disabled"));
    }

    let socket = DhtSocket::bind(config.bind).await?;
    let runtime = build_dht_runtime(&config)?;
    let (tx, rx) = mpsc::unbounded_channel();
    let join = tokio::spawn(run_dht_worker(socket, runtime, config, events, rx));
    Ok(DhtWorkerHandle {
        tx,
        join: Arc::new(Mutex::new(Some(join))),
    })
}

fn build_dht_runtime(config: &DhtRuntimeConfig) -> Result<DhtRuntime, DhtError> {
    let mut dht_config = DhtConfig::default();
    dht_config.query_timeout = config.query_timeout;
    for node in &config.bootstrap_nodes {
        dht_config.add_bootstrap_node(*node);
    }
    DhtRuntime::new(
        NodeId::new(rand::random()),
        TokenManager::with_secrets(
            Bytes::copy_from_slice(&rand::random::<[u8; 32]>()),
            Bytes::copy_from_slice(&rand::random::<[u8; 32]>()),
        ),
        dht_config,
    )
}

async fn run_dht_worker(
    socket: DhtSocket,
    mut runtime: DhtRuntime,
    config: DhtRuntimeConfig,
    events: mpsc::UnboundedSender<DhtRuntimeEvent>,
    mut commands: mpsc::UnboundedReceiver<DhtCommand>,
) {
    let mut interval = tokio::time::interval(config.tick_interval);
    let mut lookup_torrents = HashMap::<InfoHash, TorrentId>::new();
    loop {
        tokio::select! {
            _ = interval.tick() => {
                emit_timeout_events(&mut runtime, &mut lookup_torrents, &events);
            }
            command = commands.recv() => {
                let Some(command) = command else {
                    break;
                };
                if matches!(command, DhtCommand::Shutdown) {
                    break;
                }
                handle_command(command, &socket, &mut runtime, &events, &mut lookup_torrents).await;
            }
            event = socket.poll_once() => {
                match event {
                    Ok(event) => {
                        handle_socket_event(event.message, event.source, &socket, &mut runtime, &events, &mut lookup_torrents).await;
                    }
                    Err(err) => {
                        let _ = events.send(DhtRuntimeEvent::Failed { reason: err.to_string() });
                    }
                }
            }
        }
    }
}

async fn handle_command(
    command: DhtCommand,
    socket: &DhtSocket,
    runtime: &mut DhtRuntime,
    events: &mpsc::UnboundedSender<DhtRuntimeEvent>,
    lookup_torrents: &mut HashMap<InfoHash, TorrentId>,
) {
    match command {
        DhtCommand::Bootstrap => {
            send_runtime_outbound(runtime.start_bootstrap(Instant::now()), socket, events).await;
        }
        DhtCommand::GetPeers { torrent, info_hash } => {
            lookup_torrents.insert(info_hash, torrent);
            send_runtime_outbound(
                runtime.start_get_peers(info_hash, Instant::now()),
                socket,
                events,
            )
            .await;
        }
        DhtCommand::AnnouncePeer {
            torrent,
            info_hash,
            port,
            implied_port,
        } => match runtime.start_announce_peer(info_hash, port, implied_port, Instant::now()) {
            Ok(outbound) if outbound.is_empty() => {
                let _ = events.send(DhtRuntimeEvent::Failed {
                    reason: "DHT announce requires a token from a prior get_peers response"
                        .to_owned(),
                });
            }
            Ok(outbound) => {
                let nodes = send_outbound(socket, outbound, events).await;
                let nodes = u32::try_from(nodes).unwrap_or(u32::MAX);
                let _ = events.send(DhtRuntimeEvent::Announced { torrent, nodes });
            }
            Err(err) => {
                let _ = events.send(DhtRuntimeEvent::Failed {
                    reason: err.to_string(),
                });
            }
        },
        DhtCommand::Shutdown => {}
    }
}

async fn send_runtime_outbound(
    result: Result<Vec<(NodeAddr, DhtMessage)>, DhtError>,
    socket: &DhtSocket,
    events: &mpsc::UnboundedSender<DhtRuntimeEvent>,
) {
    match result {
        Ok(outbound) => {
            send_outbound(socket, outbound, events).await;
        }
        Err(err) => {
            let _ = events.send(DhtRuntimeEvent::Failed {
                reason: err.to_string(),
            });
        }
    }
}

async fn handle_socket_event(
    message: DhtMessage,
    source: SocketAddr,
    socket: &DhtSocket,
    runtime: &mut DhtRuntime,
    events: &mpsc::UnboundedSender<DhtRuntimeEvent>,
    lookup_torrents: &mut HashMap<InfoHash, TorrentId>,
) {
    match runtime.handle_message(message, NodeAddr::new(source), Instant::now()) {
        Ok(action) => {
            if let Some(response) = action.response {
                if let Err(err) = socket.send_to(&response, source).await {
                    let _ = events.send(DhtRuntimeEvent::Failed {
                        reason: err.to_string(),
                    });
                }
            }
            if let Some(event) = action.event {
                emit_dht_event(event, events, lookup_torrents);
            }
            send_outbound(socket, action.outbound, events).await;
        }
        Err(err) => {
            let _ = events.send(DhtRuntimeEvent::Failed {
                reason: err.to_string(),
            });
        }
    }
}

fn emit_timeout_events(
    runtime: &mut DhtRuntime,
    lookup_torrents: &mut HashMap<InfoHash, TorrentId>,
    events: &mpsc::UnboundedSender<DhtRuntimeEvent>,
) {
    match runtime.drain_timeouts(Instant::now()) {
        Ok(dht_events) => {
            for event in dht_events {
                emit_dht_event(event, events, lookup_torrents);
            }
        }
        Err(err) => {
            let _ = events.send(DhtRuntimeEvent::Failed {
                reason: err.to_string(),
            });
        }
    }
}

fn emit_dht_event(
    event: DhtEvent,
    events: &mpsc::UnboundedSender<DhtRuntimeEvent>,
    lookup_torrents: &mut HashMap<InfoHash, TorrentId>,
) {
    match event {
        DhtEvent::ResponseMatched {
            kind: TransactionKind::Ping,
            ..
        } => {
            let _ = events.send(DhtRuntimeEvent::Bootstrapped { nodes: 1 });
        }
        DhtEvent::PeersDiscovered { info_hash, peers } => {
            if let Some(torrent) = lookup_torrents.remove(&info_hash) {
                let _ = events.send(DhtRuntimeEvent::PeersDiscovered {
                    torrent,
                    peers: peers.into_iter().map(|peer| peer.socket_addr()).collect(),
                });
            }
        }
        DhtEvent::LookupExhausted { info_hash } => {
            if let Some(torrent) = lookup_torrents.remove(&info_hash) {
                let _ = events.send(DhtRuntimeEvent::LookupExhausted { torrent });
            }
        }
        DhtEvent::TransactionExpired {
            kind: TransactionKind::GetPeers { info_hash },
            ..
        } => {
            if let Some(torrent) = lookup_torrents.remove(&info_hash) {
                let _ = events.send(DhtRuntimeEvent::LookupExhausted { torrent });
            }
        }
        DhtEvent::ErrorReceived { code, .. } => {
            let _ = events.send(DhtRuntimeEvent::Failed {
                reason: format!("DHT error response {code}"),
            });
        }
        DhtEvent::QueryResponded { .. }
        | DhtEvent::ResponseMatched { .. }
        | DhtEvent::UnsolicitedResponse { .. }
        | DhtEvent::TransactionExpired { .. }
        | DhtEvent::ExternalIpObserved { .. } => {}
    }
}

async fn send_outbound(
    socket: &DhtSocket,
    outbound: Vec<(NodeAddr, DhtMessage)>,
    events: &mpsc::UnboundedSender<DhtRuntimeEvent>,
) -> usize {
    let mut sent = 0;
    for (target, message) in outbound {
        if let Err(err) = socket.send_to(&message, target.socket_addr()).await {
            let _ = events.send(DhtRuntimeEvent::Failed {
                reason: err.to_string(),
            });
        } else {
            sent += 1;
        }
    }
    sent
}
