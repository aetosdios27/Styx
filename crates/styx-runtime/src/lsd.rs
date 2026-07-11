//! Bounded BEP 14 Local Service Discovery packet handling.

use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use styx_proto::InfoHashV1;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::{OwnedTask, TaskKind, TorrentId};

pub const LSD_IPV4_MULTICAST: &str = "239.192.152.143:6771";
pub const LSD_IPV6_MULTICAST: &str = "[ff15::efc0:988f]:6771";
pub const MAX_LSD_PACKET_BYTES: usize = 1400;
const LSD_ANNOUNCE_INTERVAL: Duration = Duration::from_secs(5 * 60);
const DIRECT_LSD_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LsdAnnounce {
    pub info_hashes: Vec<InfoHashV1>,
    pub port: u16,
    pub cookie: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LsdDiscovery {
    pub info_hashes: Vec<InfoHashV1>,
    pub peer: SocketAddr,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum LsdError {
    #[error("LSD packet exceeds {MAX_LSD_PACKET_BYTES} bytes")]
    PacketTooLarge,
    #[error("LSD packet is not valid UTF-8")]
    InvalidUtf8,
    #[error("LSD packet has an invalid request line")]
    InvalidRequestLine,
    #[error("LSD announce port must be greater than zero")]
    InvalidPort,
    #[error("LSD announce contains an invalid info hash")]
    InvalidInfoHash,
    #[error("LSD announce must contain at least one info hash")]
    MissingInfoHash,
    #[error("LSD cookie contains invalid header characters")]
    InvalidCookie,
    #[error("LSD worker channel is closed")]
    WorkerClosed,
    #[error("LSD worker command queue is full")]
    CommandBackpressure,
    #[error("LSD worker shutdown timed out")]
    ShutdownTimeout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LsdCommand {
    Update {
        torrents: Vec<(TorrentId, InfoHashV1)>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LsdRuntimeEvent {
    PeerDiscovered {
        torrent: TorrentId,
        peer: SocketAddr,
    },
}

#[derive(Clone, Debug)]
pub struct LsdClient {
    tx: mpsc::Sender<LsdCommand>,
}

#[derive(Debug)]
pub struct LsdOwner {
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl LsdClient {
    pub fn try_send(&self, command: LsdCommand) -> Result<(), LsdError> {
        self.tx.try_send(command).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => LsdError::CommandBackpressure,
            mpsc::error::TrySendError::Closed(_) => LsdError::WorkerClosed,
        })
    }
}

impl LsdOwner {
    pub async fn shutdown(self) -> Result<(), LsdError> {
        let mut owner = self;
        if let Some(shutdown) = owner.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut join) = owner.join.take() {
            match tokio::time::timeout(DIRECT_LSD_SHUTDOWN_TIMEOUT, &mut join).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return Err(LsdError::WorkerClosed),
                Err(_) => {
                    join.abort();
                    return Err(LsdError::ShutdownTimeout);
                }
            }
        }
        Ok(())
    }

    pub fn into_task(mut self) -> OwnedTask {
        let join = self
            .join
            .take()
            .expect("LSD owner must contain exactly one worker task");
        let shutdown = self
            .shutdown
            .take()
            .expect("LSD owner must contain exactly one shutdown capability");
        OwnedTask::with_shutdown(TaskKind::Lsd, join, shutdown)
    }
}

impl Drop for LsdOwner {
    fn drop(&mut self) {
        if let Some(join) = &self.join {
            join.abort();
        }
    }
}

pub fn spawn_lsd_worker(
    listen_port: u16,
    command_capacity: NonZeroUsize,
    events: mpsc::Sender<LsdRuntimeEvent>,
) -> Option<(LsdClient, LsdOwner)> {
    if listen_port == 0 {
        return None;
    }
    let socket = std::net::UdpSocket::bind("0.0.0.0:6771").ok()?;
    socket.set_nonblocking(true).ok()?;
    socket
        .join_multicast_v4(
            &std::net::Ipv4Addr::new(239, 192, 152, 143),
            &std::net::Ipv4Addr::UNSPECIFIED,
        )
        .ok()?;
    let socket = tokio::net::UdpSocket::from_std(socket).ok()?;
    let (tx, rx) = mpsc::channel(command_capacity.get());
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(run_lsd_worker(socket, listen_port, events, rx, shutdown_rx));
    Some((
        LsdClient { tx },
        LsdOwner {
            shutdown: Some(shutdown_tx),
            join: Some(join),
        },
    ))
}

async fn run_lsd_worker(
    socket: tokio::net::UdpSocket,
    listen_port: u16,
    events: mpsc::Sender<LsdRuntimeEvent>,
    mut commands: mpsc::Receiver<LsdCommand>,
    mut shutdown: oneshot::Receiver<()>,
) {
    let cookie = format!("{:032x}", rand::random::<u128>());
    let target: SocketAddr = LSD_IPV4_MULTICAST.parse().expect("constant is valid");
    let mut torrents = Vec::<(TorrentId, InfoHashV1)>::new();
    let mut last_announce = None::<Instant>;
    let mut interval = tokio::time::interval(LSD_ANNOUNCE_INTERVAL);
    let mut buffer = [0; MAX_LSD_PACKET_BYTES + 1];
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = interval.tick() => {
                if !torrents.is_empty()
                    && last_announce.is_none_or(|last| last.elapsed() >= Duration::from_secs(60))
                {
                    let hashes: Vec<_> = torrents.iter().map(|(_, hash)| *hash).collect();
                    for packet in encode_lsd_packets(&hashes, listen_port, &cookie) {
                        let _ = socket.send_to(&packet, target).await;
                    }
                    last_announce = Some(Instant::now());
                }
            }
            command = commands.recv() => {
                match command {
                    Some(LsdCommand::Update { torrents: updated }) => torrents = updated,
                    None => break,
                }
            }
            received = socket.recv_from(&mut buffer) => {
                let Ok((length, source)) = received else { continue; };
                if length > MAX_LSD_PACKET_BYTES { continue; }
                let Ok(discovery) = decode_lsd_announce(&buffer[..length], source, &cookie) else {
                    continue;
                };
                for info_hash in discovery.info_hashes {
                    for (torrent, active_hash) in &torrents {
                        if *active_hash == info_hash {
                            let _ = events.send(LsdRuntimeEvent::PeerDiscovered {
                                torrent: *torrent,
                                peer: discovery.peer,
                            }).await;
                        }
                    }
                }
            }
        }
    }
}

fn encode_lsd_packets(info_hashes: &[InfoHashV1], port: u16, cookie: &str) -> Vec<Vec<u8>> {
    let mut packets = Vec::new();
    let mut batch = Vec::new();
    for info_hash in info_hashes {
        batch.push(*info_hash);
        let announce = LsdAnnounce {
            info_hashes: batch.clone(),
            port,
            cookie: cookie.to_owned(),
        };
        if matches!(
            encode_lsd_announce(&announce),
            Err(LsdError::PacketTooLarge)
        ) {
            batch.pop();
            if !batch.is_empty() {
                let packet = encode_lsd_announce(&LsdAnnounce {
                    info_hashes: std::mem::take(&mut batch),
                    port,
                    cookie: cookie.to_owned(),
                })
                .expect("a previously fitting LSD batch remains valid");
                packets.push(packet);
            }
            batch.push(*info_hash);
        }
    }
    if !batch.is_empty() {
        if let Ok(packet) = encode_lsd_announce(&LsdAnnounce {
            info_hashes: batch,
            port,
            cookie: cookie.to_owned(),
        }) {
            packets.push(packet);
        }
    }
    packets
}

pub fn encode_lsd_announce(announce: &LsdAnnounce) -> Result<Vec<u8>, LsdError> {
    if announce.port == 0 {
        return Err(LsdError::InvalidPort);
    }
    if announce.info_hashes.is_empty() {
        return Err(LsdError::MissingInfoHash);
    }
    if announce.cookie.is_empty() || announce.cookie.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(LsdError::InvalidCookie);
    }
    let mut packet = format!(
        "BT-SEARCH * HTTP/1.1\r\nHost: {LSD_IPV4_MULTICAST}\r\nPort: {}\r\nCookie: {}\r\n",
        announce.port, announce.cookie
    );
    for info_hash in &announce.info_hashes {
        use std::fmt::Write as _;
        packet.push_str("Infohash: ");
        for byte in info_hash.as_bytes() {
            write!(&mut packet, "{byte:02X}").expect("writing to String cannot fail");
        }
        packet.push_str("\r\n");
    }
    packet.push_str("\r\n");
    if packet.len() > MAX_LSD_PACKET_BYTES {
        return Err(LsdError::PacketTooLarge);
    }
    Ok(packet.into_bytes())
}

pub fn decode_lsd_announce(
    packet: &[u8],
    source: SocketAddr,
    own_cookie: &str,
) -> Result<LsdDiscovery, LsdError> {
    if packet.len() > MAX_LSD_PACKET_BYTES {
        return Err(LsdError::PacketTooLarge);
    }
    let text = std::str::from_utf8(packet).map_err(|_| LsdError::InvalidUtf8)?;
    let mut lines = text.split("\r\n");
    if lines.next() != Some("BT-SEARCH * HTTP/1.1") {
        return Err(LsdError::InvalidRequestLine);
    }
    let mut port = None;
    let mut cookie = None;
    let mut info_hashes = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("port") {
            port = value.parse::<u16>().ok().filter(|port| *port != 0);
        } else if name.eq_ignore_ascii_case("cookie") {
            cookie = Some(value);
        } else if name.eq_ignore_ascii_case("infohash") {
            info_hashes.push(parse_info_hash(value)?);
        }
    }
    let port = port.ok_or(LsdError::InvalidPort)?;
    if info_hashes.is_empty() {
        return Err(LsdError::MissingInfoHash);
    }
    info_hashes.sort_unstable_by_key(|hash| *hash.as_bytes());
    info_hashes.dedup();
    if cookie == Some(own_cookie) {
        info_hashes.clear();
    }
    Ok(LsdDiscovery {
        info_hashes,
        peer: SocketAddr::new(source.ip(), port),
    })
}

fn parse_info_hash(value: &str) -> Result<InfoHashV1, LsdError> {
    if value.len() != 40 {
        return Err(LsdError::InvalidInfoHash);
    }
    let mut bytes = [0; 20];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0]).ok_or(LsdError::InvalidInfoHash)?;
        let low = hex_nibble(pair[1]).ok_or(LsdError::InvalidInfoHash)?;
        bytes[index] = (high << 4) | low;
    }
    Ok(InfoHashV1::new(bytes))
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{future, time::Duration};

    use super::*;
    use crate::{ShutdownMode, TaskExit, TaskRegistry};

    #[test]
    fn lsd_client_reports_backpressure_when_command_channel_is_full() {
        let (tx, _rx) = mpsc::channel(1);
        let client = LsdClient { tx };
        let update = || LsdCommand::Update {
            torrents: Vec::new(),
        };
        client.try_send(update()).unwrap();

        let error = client.try_send(update()).unwrap_err();

        assert_eq!(error, LsdError::CommandBackpressure);
    }

    #[test]
    fn lsd_client_reports_worker_closed_when_command_channel_is_closed() {
        let (tx, rx) = mpsc::channel(1);
        let client = LsdClient { tx };
        drop(rx);

        let error = client
            .try_send(LsdCommand::Update {
                torrents: Vec::new(),
            })
            .unwrap_err();

        assert_eq!(error, LsdError::WorkerClosed);
    }

    #[tokio::test]
    async fn lsd_owner_into_task_preserves_cooperative_registry_shutdown() {
        let (command_tx, command_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            let _command_rx = command_rx;
            let _ = shutdown_rx.await;
        });
        let owner = LsdOwner {
            shutdown: Some(shutdown_tx),
            join: Some(join),
        };
        let client = LsdClient { tx: command_tx };
        let mut registry = TaskRegistry::default();
        registry.register(owner.into_task());
        client
            .try_send(LsdCommand::Update {
                torrents: Vec::new(),
            })
            .unwrap();

        let exits = registry
            .shutdown(
                ShutdownMode::Clean,
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
            .await;

        assert_eq!(exits[&TaskKind::Lsd], vec![TaskExit::Graceful]);
    }

    #[tokio::test]
    async fn dropping_lsd_owner_aborts_worker_and_closes_client() {
        let (command_tx, command_rx) = mpsc::channel(1);
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            let _command_rx = command_rx;
            future::pending::<()>().await;
        });
        let owner = LsdOwner {
            shutdown: Some(shutdown_tx),
            join: Some(join),
        };
        let client = LsdClient { tx: command_tx };
        drop(owner);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if client
                    .try_send(LsdCommand::Update {
                        torrents: Vec::new(),
                    })
                    .is_err_and(|error| error == LsdError::WorkerClosed)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("owner drop must abort the LSD worker and close clients");
    }

    #[tokio::test(start_paused = true)]
    async fn lsd_owner_shutdown_is_bounded_when_event_delivery_is_stalled() {
        let (events, _receiver) = mpsc::channel(1);
        events
            .try_send(LsdRuntimeEvent::PeerDiscovered {
                torrent: TorrentId::new(InfoHashV1::new([1; 20])),
                peer: "127.0.0.1:6881".parse().unwrap(),
            })
            .unwrap();
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            let _ = events
                .send(LsdRuntimeEvent::PeerDiscovered {
                    torrent: TorrentId::new(InfoHashV1::new([2; 20])),
                    peer: "127.0.0.1:6882".parse().unwrap(),
                })
                .await;
        });
        let owner = LsdOwner {
            shutdown: Some(shutdown_tx),
            join: Some(join),
        };
        let shutdown = tokio::spawn(owner.shutdown());
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(1)).await;

        let result = shutdown.await.unwrap();

        assert_eq!(result, Err(LsdError::ShutdownTimeout));
    }
}
