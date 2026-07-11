use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use bytes::Bytes;
use styx_dht::{
    DhtConfig, DhtError, DhtEvent, DhtMessage, DhtRuntime, DhtSocket, InfoHash, NodeAddr, NodeId,
    TokenManager, TransactionKind,
};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::{OwnedTask, RuntimeError, TaskKind, TorrentId};

const DEFAULT_METADATA_SIZE_LIMIT: u64 = 8 * 1024 * 1024;
const DEFAULT_METADATA_REQUEST_LIMIT: u32 = 512;
const DEFAULT_DHT_TICK_INTERVAL: Duration = Duration::from_millis(250);
const DIRECT_DHT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DhtRuntimeConfig {
    pub enabled: bool,
    pub bind: SocketAddr,
    pub bootstrap_nodes: Vec<SocketAddr>,
    pub query_timeout: Duration,
    pub command_capacity: usize,
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
pub struct DhtClient {
    tx: mpsc::Sender<DhtCommand>,
}

#[derive(Debug)]
pub struct DhtOwner {
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl Default for DhtRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 6881),
            bootstrap_nodes: Vec::new(),
            query_timeout: Duration::from_secs(15),
            command_capacity: 256,
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
        if self.command_capacity == 0 {
            return Err(RuntimeError::InvalidConfig(
                "dht command capacity must be greater than zero",
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

impl DhtClient {
    pub fn try_send(&self, command: DhtCommand) -> Result<(), RuntimeError> {
        self.tx.try_send(command).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => RuntimeError::Backpressure {
                stage: "dht_command",
            },
            mpsc::error::TrySendError::Closed(_) => RuntimeError::Cancelled,
        })
    }
}

impl DhtOwner {
    pub async fn shutdown(self) -> Result<(), RuntimeError> {
        let mut owner = self;
        if let Some(shutdown) = owner.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut join) = owner.join.take() {
            match tokio::time::timeout(DIRECT_DHT_SHUTDOWN_TIMEOUT, &mut join).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return Err(RuntimeError::Cancelled),
                Err(_) => {
                    join.abort();
                    return Err(RuntimeError::Timeout {
                        stage: "dht_shutdown",
                    });
                }
            }
        }
        Ok(())
    }

    pub fn into_task(mut self) -> OwnedTask {
        let join = self
            .join
            .take()
            .expect("DHT owner must contain exactly one worker task");
        let shutdown = self
            .shutdown
            .take()
            .expect("DHT owner must contain exactly one shutdown capability");
        OwnedTask::with_shutdown(TaskKind::Dht, join, shutdown)
    }
}

impl Drop for DhtOwner {
    fn drop(&mut self) {
        if let Some(join) = &self.join {
            join.abort();
        }
    }
}

pub async fn spawn_dht_worker(
    config: DhtRuntimeConfig,
    events: mpsc::Sender<DhtRuntimeEvent>,
) -> Result<(DhtClient, DhtOwner), RuntimeError> {
    config.validate()?;
    if !config.enabled {
        return Err(RuntimeError::InvalidConfig("dht worker is disabled"));
    }

    let socket = DhtSocket::bind(config.bind).await?;
    let runtime = build_dht_runtime(&config)?;
    let (tx, rx) = mpsc::channel(config.command_capacity);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(run_dht_worker(
        socket,
        runtime,
        config,
        events,
        rx,
        shutdown_rx,
    ));
    Ok((
        DhtClient { tx },
        DhtOwner {
            shutdown: Some(shutdown_tx),
            join: Some(join),
        },
    ))
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
    events: mpsc::Sender<DhtRuntimeEvent>,
    mut commands: mpsc::Receiver<DhtCommand>,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut interval = tokio::time::interval(config.tick_interval);
    let mut lookup_torrents = HashMap::<InfoHash, TorrentId>::new();
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                break;
            }
            _ = interval.tick() => {
                emit_timeout_events(&mut runtime, &mut lookup_torrents, &events).await;
            }
            command = commands.recv() => {
                let Some(command) = command else {
                    break;
                };
                handle_command(command, &socket, &mut runtime, &events, &mut lookup_torrents).await;
            }
            event = socket.poll_once() => {
                match event {
                    Ok(event) => {
                        handle_socket_event(event.message, event.source, &socket, &mut runtime, &events, &mut lookup_torrents).await;
                    }
                    Err(err) => {
                        let _ = events.send(DhtRuntimeEvent::Failed { reason: err.to_string() }).await;
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
    events: &mpsc::Sender<DhtRuntimeEvent>,
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
                let _ = events
                    .send(DhtRuntimeEvent::Failed {
                        reason: "DHT announce requires a token from a prior get_peers response"
                            .to_owned(),
                    })
                    .await;
            }
            Ok(outbound) => {
                let nodes = send_outbound(socket, outbound, events).await;
                let nodes = u32::try_from(nodes).unwrap_or(u32::MAX);
                let _ = events
                    .send(DhtRuntimeEvent::Announced { torrent, nodes })
                    .await;
            }
            Err(err) => {
                let _ = events
                    .send(DhtRuntimeEvent::Failed {
                        reason: err.to_string(),
                    })
                    .await;
            }
        },
    }
}

async fn send_runtime_outbound(
    result: Result<Vec<(NodeAddr, DhtMessage)>, DhtError>,
    socket: &DhtSocket,
    events: &mpsc::Sender<DhtRuntimeEvent>,
) {
    match result {
        Ok(outbound) => {
            send_outbound(socket, outbound, events).await;
        }
        Err(err) => {
            let _ = events
                .send(DhtRuntimeEvent::Failed {
                    reason: err.to_string(),
                })
                .await;
        }
    }
}

async fn handle_socket_event(
    message: DhtMessage,
    source: SocketAddr,
    socket: &DhtSocket,
    runtime: &mut DhtRuntime,
    events: &mpsc::Sender<DhtRuntimeEvent>,
    lookup_torrents: &mut HashMap<InfoHash, TorrentId>,
) {
    match runtime.handle_message(message, NodeAddr::new(source), Instant::now()) {
        Ok(action) => {
            if let Some(response) = action.response {
                if let Err(err) = socket.send_to(&response, source).await {
                    let _ = events
                        .send(DhtRuntimeEvent::Failed {
                            reason: err.to_string(),
                        })
                        .await;
                }
            }
            if let Some(event) = action.event {
                emit_dht_event(event, events, lookup_torrents).await;
            }
            send_outbound(socket, action.outbound, events).await;
        }
        Err(err) => {
            let _ = events
                .send(DhtRuntimeEvent::Failed {
                    reason: err.to_string(),
                })
                .await;
        }
    }
}

async fn emit_timeout_events(
    runtime: &mut DhtRuntime,
    lookup_torrents: &mut HashMap<InfoHash, TorrentId>,
    events: &mpsc::Sender<DhtRuntimeEvent>,
) {
    match runtime.drain_timeouts(Instant::now()) {
        Ok(dht_events) => {
            for event in dht_events {
                emit_dht_event(event, events, lookup_torrents).await;
            }
        }
        Err(err) => {
            let _ = events
                .send(DhtRuntimeEvent::Failed {
                    reason: err.to_string(),
                })
                .await;
        }
    }
}

async fn emit_dht_event(
    event: DhtEvent,
    events: &mpsc::Sender<DhtRuntimeEvent>,
    lookup_torrents: &mut HashMap<InfoHash, TorrentId>,
) {
    match event {
        DhtEvent::ResponseMatched {
            kind: TransactionKind::Ping,
            ..
        } => {
            let _ = events
                .send(DhtRuntimeEvent::Bootstrapped { nodes: 1 })
                .await;
        }
        DhtEvent::PeersDiscovered { info_hash, peers } => {
            if let Some(torrent) = lookup_torrents.remove(&info_hash) {
                let _ = events
                    .send(DhtRuntimeEvent::PeersDiscovered {
                        torrent,
                        peers: peers.into_iter().map(|peer| peer.socket_addr()).collect(),
                    })
                    .await;
            }
        }
        DhtEvent::LookupExhausted { info_hash } => {
            if let Some(torrent) = lookup_torrents.remove(&info_hash) {
                let _ = events
                    .send(DhtRuntimeEvent::LookupExhausted { torrent })
                    .await;
            }
        }
        DhtEvent::TransactionExpired {
            kind: TransactionKind::GetPeers { info_hash },
            ..
        } => {
            if let Some(torrent) = lookup_torrents.remove(&info_hash) {
                let _ = events
                    .send(DhtRuntimeEvent::LookupExhausted { torrent })
                    .await;
            }
        }
        DhtEvent::ErrorReceived { code, .. } => {
            let _ = events
                .send(DhtRuntimeEvent::Failed {
                    reason: format!("DHT error response {code}"),
                })
                .await;
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
    events: &mpsc::Sender<DhtRuntimeEvent>,
) -> usize {
    let mut sent = 0;
    for (target, message) in outbound {
        if let Err(err) = socket.send_to(&message, target.socket_addr()).await {
            let _ = events
                .send(DhtRuntimeEvent::Failed {
                    reason: err.to_string(),
                })
                .await;
        } else {
            sent += 1;
        }
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dht_client_reports_backpressure_when_command_channel_is_full() {
        let (tx, _rx) = mpsc::channel(1);
        let client = DhtClient { tx };
        client.try_send(DhtCommand::Bootstrap).unwrap();

        let error = client.try_send(DhtCommand::Bootstrap).unwrap_err();

        assert!(matches!(
            error,
            RuntimeError::Backpressure {
                stage: "dht_command"
            }
        ));
    }

    #[test]
    fn dht_client_reports_cancellation_when_command_channel_is_closed() {
        let (tx, rx) = mpsc::channel(1);
        let client = DhtClient { tx };
        drop(rx);

        let error = client.try_send(DhtCommand::Bootstrap).unwrap_err();

        assert!(matches!(error, RuntimeError::Cancelled));
    }

    #[tokio::test(start_paused = true)]
    async fn dht_owner_shutdown_is_bounded_when_event_delivery_is_stalled() {
        let (events, _receiver) = mpsc::channel(1);
        events
            .try_send(DhtRuntimeEvent::Failed {
                reason: "fill queue".into(),
            })
            .unwrap();
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            let _ = events
                .send(DhtRuntimeEvent::Failed {
                    reason: "blocked delivery".into(),
                })
                .await;
        });
        let owner = DhtOwner {
            shutdown: Some(shutdown_tx),
            join: Some(join),
        };
        let shutdown = tokio::spawn(owner.shutdown());
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(1)).await;

        let result = shutdown.await.unwrap();

        assert!(matches!(
            result,
            Err(RuntimeError::Timeout {
                stage: "dht_shutdown"
            })
        ));
    }
}
