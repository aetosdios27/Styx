//! Bounded BEP 14 Local Service Discovery packet handling.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use styx_proto::InfoHashV1;
use tokio::sync::{mpsc, Mutex};

use crate::TorrentId;

pub const LSD_IPV4_MULTICAST: &str = "239.192.152.143:6771";
pub const LSD_IPV6_MULTICAST: &str = "[ff15::efc0:988f]:6771";
pub const MAX_LSD_PACKET_BYTES: usize = 1400;
const LSD_ANNOUNCE_INTERVAL: Duration = Duration::from_secs(5 * 60);

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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LsdCommand {
    Update {
        torrents: Vec<(TorrentId, InfoHashV1)>,
    },
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LsdRuntimeEvent {
    PeerDiscovered {
        torrent: TorrentId,
        peer: SocketAddr,
    },
}

#[derive(Clone, Debug)]
pub struct LsdWorkerHandle {
    tx: mpsc::UnboundedSender<LsdCommand>,
    join: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl LsdWorkerHandle {
    pub fn send(&self, command: LsdCommand) -> Result<(), LsdError> {
        self.tx.send(command).map_err(|_| LsdError::WorkerClosed)
    }

    pub async fn shutdown(self) {
        let _ = self.tx.send(LsdCommand::Shutdown);
        if let Some(join) = self.join.lock().await.take() {
            let _ = join.await;
        }
    }
}

pub fn spawn_lsd_worker(
    listen_port: u16,
    events: mpsc::UnboundedSender<LsdRuntimeEvent>,
) -> Option<LsdWorkerHandle> {
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
    let (tx, rx) = mpsc::unbounded_channel();
    let join = tokio::spawn(run_lsd_worker(socket, listen_port, events, rx));
    Some(LsdWorkerHandle {
        tx,
        join: Arc::new(Mutex::new(Some(join))),
    })
}

async fn run_lsd_worker(
    socket: tokio::net::UdpSocket,
    listen_port: u16,
    events: mpsc::UnboundedSender<LsdRuntimeEvent>,
    mut commands: mpsc::UnboundedReceiver<LsdCommand>,
) {
    let cookie = format!("{:032x}", rand::random::<u128>());
    let target: SocketAddr = LSD_IPV4_MULTICAST.parse().expect("constant is valid");
    let mut torrents = Vec::<(TorrentId, InfoHashV1)>::new();
    let mut last_announce = None::<Instant>;
    let mut interval = tokio::time::interval(LSD_ANNOUNCE_INTERVAL);
    let mut buffer = [0; MAX_LSD_PACKET_BYTES + 1];
    loop {
        tokio::select! {
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
                    Some(LsdCommand::Shutdown) | None => break,
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
                            });
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
