use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use bytes::{BufMut, Bytes, BytesMut};

use crate::{
    AnnounceEvent, AnnounceRequest, ScrapeRequest, ScrapeResponse, ScrapeStats, TrackerError,
    TrackerPeer,
};

const UDP_TRACKER_PROTOCOL_ID: i64 = 0x41727101980;
const CONNECT_REQUEST_LEN: usize = 16;
const CONNECT_RESPONSE_MIN_LEN: usize = 16;
const ANNOUNCE_REQUEST_LEN: usize = 98;
const ANNOUNCE_RESPONSE_MIN_LEN: usize = 20;
const IPV4_COMPACT_PEER_STRIDE: usize = 6;
const SCRAPE_RESPONSE_PREFIX_LEN: usize = 8;
const SCRAPE_STATS_STRIDE: usize = 12;
const CONNECTION_ID_TTL: Duration = Duration::from_secs(60);

/// UDP tracker action code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UdpAction {
    /// BEP 15 connect action.
    Connect,
    /// BEP 15 announce action.
    Announce,
    /// BEP 15 scrape action.
    Scrape,
    /// BEP 15 error action.
    Error,
}

impl UdpAction {
    /// Return the wire action code.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::Connect => 0,
            Self::Announce => 1,
            Self::Scrape => 2,
            Self::Error => 3,
        }
    }

    /// Parse a wire action code.
    ///
    /// # Errors
    ///
    /// Returns [`TrackerError::UnexpectedUdpAction`] when `code` is unknown.
    pub fn try_from_code(code: i32) -> Result<Self, TrackerError> {
        match code {
            0 => Ok(Self::Connect),
            1 => Ok(Self::Announce),
            2 => Ok(Self::Scrape),
            3 => Ok(Self::Error),
            _ => Err(TrackerError::UnexpectedUdpAction {
                expected: -1,
                actual: code,
            }),
        }
    }
}

/// Parsed UDP connect response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UdpConnectResponse {
    /// Transaction id echoed by the tracker.
    pub transaction_id: i32,
    /// Connection id issued by the tracker.
    pub connection_id: i64,
}

/// Parsed UDP announce response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UdpAnnounceResponse {
    /// Transaction id echoed by the tracker.
    pub transaction_id: i32,
    /// Seconds until the next announce.
    pub interval: u32,
    /// Number of leechers.
    pub leechers: u32,
    /// Number of seeders.
    pub seeders: u32,
    /// Peers returned by the tracker.
    pub peers: Vec<TrackerPeer>,
}

/// Cached UDP connection id with receipt timestamp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionIdCache {
    connection_id: i64,
    received_at: Instant,
}

/// Async UDP tracker client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UdpTrackerClient {
    timeout: Duration,
}

impl UdpTrackerClient {
    /// Create a UDP tracker client with per-request timeout.
    #[must_use]
    pub const fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Send one connect request and wait for one connect response.
    ///
    /// # Errors
    ///
    /// Returns [`TrackerError`] for socket IO, timeout IO, or malformed response
    /// packets.
    pub async fn connect_once(
        self,
        tracker: SocketAddr,
        transaction_id: i32,
    ) -> Result<UdpConnectResponse, TrackerError> {
        let bind_addr = if tracker.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = tokio::net::UdpSocket::bind(bind_addr).await?;
        let request = encode_connect_request(transaction_id);
        socket.send_to(&request, tracker).await?;

        let mut buf = [0; 2048];
        let received = tokio::time::timeout(self.timeout, socket.recv_from(&mut buf))
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "UDP tracker request timed out",
                )
            })??;
        let (len, _addr) = received;
        decode_connect_response(&buf[..len], transaction_id)
    }
}

impl ConnectionIdCache {
    /// Create a new connection-id cache entry.
    #[must_use]
    pub const fn new(connection_id: i64, received_at: Instant) -> Self {
        Self {
            connection_id,
            received_at,
        }
    }

    /// Return the cached connection id if it is still valid at `now`.
    ///
    /// # Errors
    ///
    /// Returns [`TrackerError::ConnectionIdExpired`] when the one-minute BEP 15
    /// validity window has elapsed.
    pub fn connection_id_at(self, now: Instant) -> Result<i64, TrackerError> {
        if now.duration_since(self.received_at) >= CONNECTION_ID_TTL {
            return Err(TrackerError::ConnectionIdExpired);
        }
        Ok(self.connection_id)
    }
}

/// Encode a UDP tracker connect request.
#[must_use]
pub fn encode_connect_request(transaction_id: i32) -> Bytes {
    let mut bytes = BytesMut::with_capacity(CONNECT_REQUEST_LEN);
    bytes.put_i64(UDP_TRACKER_PROTOCOL_ID);
    bytes.put_i32(UdpAction::Connect.code());
    bytes.put_i32(transaction_id);
    bytes.freeze()
}

/// Decode a UDP tracker connect response.
///
/// # Errors
///
/// Returns [`TrackerError`] for short packets, unexpected action, or transaction
/// id mismatch.
pub fn decode_connect_response(
    input: &[u8],
    expected_transaction_id: i32,
) -> Result<UdpConnectResponse, TrackerError> {
    require_min_len("connect response", input, CONNECT_RESPONSE_MIN_LEN)?;
    validate_action(input, UdpAction::Connect)?;
    let transaction_id = read_i32(input, 4);
    validate_transaction_id(transaction_id, expected_transaction_id)?;
    Ok(UdpConnectResponse {
        transaction_id,
        connection_id: read_i64(input, 8),
    })
}

/// Encode a UDP tracker announce request.
///
/// # Errors
///
/// Returns [`TrackerError::InvalidIntegerRange`] if byte counters cannot fit
/// BEP 15 signed 64-bit fields.
pub fn encode_announce_request(
    connection_id: i64,
    transaction_id: i32,
    request: &AnnounceRequest,
    ip_address: Ipv4Addr,
) -> Result<Bytes, TrackerError> {
    let mut bytes = BytesMut::with_capacity(ANNOUNCE_REQUEST_LEN);
    bytes.put_i64(connection_id);
    bytes.put_i32(UdpAction::Announce.code());
    bytes.put_i32(transaction_id);
    bytes.extend_from_slice(request.info_hash.as_bytes());
    bytes.extend_from_slice(request.peer_id.as_bytes());
    bytes.put_i64(i64_counter(request.downloaded, "downloaded")?);
    bytes.put_i64(i64_counter(request.left, "left")?);
    bytes.put_i64(i64_counter(request.uploaded, "uploaded")?);
    bytes.put_i32(event_code(request.event));
    bytes.extend_from_slice(&ip_address.octets());
    bytes.put_i32(request.key.unwrap_or(0) as i32);
    bytes.put_i32(request.numwant.unwrap_or(-1_i32 as u32) as i32);
    bytes.put_u16(request.port);
    Ok(bytes.freeze())
}

/// Decode a UDP tracker announce response with IPv4 compact peers.
///
/// # Errors
///
/// Returns [`TrackerError`] for malformed packet prefixes, transaction
/// mismatch, invalid integer ranges, or invalid compact peer lengths.
pub fn decode_announce_response(
    input: &[u8],
    expected_transaction_id: i32,
) -> Result<UdpAnnounceResponse, TrackerError> {
    require_min_len("announce response", input, ANNOUNCE_RESPONSE_MIN_LEN)?;
    validate_action(input, UdpAction::Announce)?;
    let transaction_id = read_i32(input, 4);
    validate_transaction_id(transaction_id, expected_transaction_id)?;
    let interval = u32_field(read_i32(input, 8), "interval")?;
    let leechers = u32_field(read_i32(input, 12), "leechers")?;
    let seeders = u32_field(read_i32(input, 16), "seeders")?;
    let peers = parse_compact_ipv4_peers(&input[ANNOUNCE_RESPONSE_MIN_LEN..])?;

    Ok(UdpAnnounceResponse {
        transaction_id,
        interval,
        leechers,
        seeders,
        peers,
    })
}

/// Encode a UDP tracker scrape request.
#[must_use]
pub fn encode_scrape_request(
    connection_id: i64,
    transaction_id: i32,
    request: &ScrapeRequest,
) -> Bytes {
    let mut bytes = BytesMut::with_capacity(16 + request.info_hashes.len() * 20);
    bytes.put_i64(connection_id);
    bytes.put_i32(UdpAction::Scrape.code());
    bytes.put_i32(transaction_id);
    for info_hash in &request.info_hashes {
        bytes.extend_from_slice(info_hash.as_bytes());
    }
    bytes.freeze()
}

/// Decode a UDP tracker scrape response.
///
/// # Errors
///
/// Returns [`TrackerError`] for malformed packet prefixes, transaction
/// mismatch, invalid stat counts, or invalid integer ranges.
pub fn decode_scrape_response(
    input: &[u8],
    expected_transaction_id: i32,
    requested_hashes: &[styx_proto::InfoHashV1],
) -> Result<ScrapeResponse, TrackerError> {
    require_min_len("scrape response", input, SCRAPE_RESPONSE_PREFIX_LEN)?;
    validate_action(input, UdpAction::Scrape)?;
    let transaction_id = read_i32(input, 4);
    validate_transaction_id(transaction_id, expected_transaction_id)?;

    let stats_bytes = &input[SCRAPE_RESPONSE_PREFIX_LEN..];
    let expected_len = requested_hashes.len() * SCRAPE_STATS_STRIDE;
    if stats_bytes.len() != expected_len {
        return Err(TrackerError::InvalidUdpPacket {
            context: "scrape response stats",
            actual: stats_bytes.len(),
            minimum: expected_len,
        });
    }

    let files = requested_hashes
        .iter()
        .copied()
        .zip(stats_bytes.chunks_exact(SCRAPE_STATS_STRIDE))
        .map(|(info_hash, chunk)| {
            Ok((
                info_hash,
                ScrapeStats {
                    complete: u32_field(read_i32(chunk, 0), "complete")?,
                    downloaded: u32_field(read_i32(chunk, 4), "downloaded")?,
                    incomplete: u32_field(read_i32(chunk, 8), "incomplete")?,
                },
            ))
        })
        .collect::<Result<Vec<_>, TrackerError>>()?;

    Ok(ScrapeResponse { files })
}

/// Return the BEP 15 retry delay for attempt `n`.
///
/// # Errors
///
/// Returns [`TrackerError::InvalidIntegerRange`] for attempts greater than 8.
pub fn retry_delay(attempt: u32) -> Result<Duration, TrackerError> {
    if attempt > 8 {
        return Err(TrackerError::InvalidIntegerRange {
            field: "retry attempt",
        });
    }
    Ok(Duration::from_secs(15 * (1_u64 << attempt)))
}

fn require_min_len(
    context: &'static str,
    input: &[u8],
    minimum: usize,
) -> Result<(), TrackerError> {
    if input.len() < minimum {
        return Err(TrackerError::InvalidUdpPacket {
            context,
            actual: input.len(),
            minimum,
        });
    }
    Ok(())
}

fn validate_action(input: &[u8], expected: UdpAction) -> Result<(), TrackerError> {
    let actual = read_i32(input, 0);
    if actual == expected.code() {
        return Ok(());
    }
    Err(TrackerError::UnexpectedUdpAction {
        expected: expected.code(),
        actual,
    })
}

fn validate_transaction_id(actual: i32, expected: i32) -> Result<(), TrackerError> {
    if actual == expected {
        return Ok(());
    }
    Err(TrackerError::TransactionIdMismatch { expected, actual })
}

fn event_code(event: Option<AnnounceEvent>) -> i32 {
    match event {
        None => 0,
        Some(AnnounceEvent::Completed) => 1,
        Some(AnnounceEvent::Started) => 2,
        Some(AnnounceEvent::Stopped) => 3,
    }
}

fn parse_compact_ipv4_peers(input: &[u8]) -> Result<Vec<TrackerPeer>, TrackerError> {
    if !input.len().is_multiple_of(IPV4_COMPACT_PEER_STRIDE) {
        return Err(TrackerError::InvalidCompactPeerLength {
            actual: input.len(),
            stride: IPV4_COMPACT_PEER_STRIDE,
        });
    }

    input
        .chunks_exact(IPV4_COMPACT_PEER_STRIDE)
        .map(|chunk| {
            let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);
            let port = u16::from_be_bytes([chunk[4], chunk[5]]);
            Ok(TrackerPeer {
                addr: SocketAddr::new(IpAddr::V4(ip), port),
                peer_id: None,
            })
        })
        .collect()
}

fn u32_field(value: i32, field: &'static str) -> Result<u32, TrackerError> {
    u32::try_from(value).map_err(|_| TrackerError::InvalidIntegerRange { field })
}

fn i64_counter(value: u64, field: &'static str) -> Result<i64, TrackerError> {
    i64::try_from(value).map_err(|_| TrackerError::InvalidIntegerRange { field })
}

fn read_i32(input: &[u8], offset: usize) -> i32 {
    i32::from_be_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
    ])
}

fn read_i64(input: &[u8], offset: usize) -> i64 {
    i64::from_be_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
        input[offset + 4],
        input[offset + 5],
        input[offset + 6],
        input[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::{Duration, Instant};

    use styx_proto::{InfoHashV1, PeerId};

    use crate::{
        decode_announce_response, decode_connect_response, decode_scrape_response,
        encode_announce_request, encode_connect_request, encode_scrape_request, retry_delay,
        AnnounceEvent, AnnounceRequest, ConnectionIdCache, ScrapeRequest, ScrapeStats,
        TrackerError, TrackerPeer, UdpAction, UdpTrackerClient,
    };

    fn announce_request() -> AnnounceRequest {
        AnnounceRequest {
            info_hash: InfoHashV1::new([1; 20]),
            peer_id: PeerId::new([2; 20]),
            port: 6881,
            uploaded: 10,
            downloaded: 20,
            left: 30,
            event: Some(AnnounceEvent::Started),
            compact: true,
            numwant: Some(50),
            key: Some(99),
        }
    }

    #[test]
    fn udp_action_parses_known_action_codes() {
        assert_eq!(UdpAction::try_from_code(1).unwrap(), UdpAction::Announce);
    }

    #[test]
    fn udp_action_rejects_unknown_action_code() {
        let err = UdpAction::try_from_code(99).unwrap_err();

        assert!(matches!(
            err,
            TrackerError::UnexpectedUdpAction {
                expected: -1,
                actual: 99
            }
        ));
    }

    #[test]
    fn encode_connect_request_matches_bep15_layout() {
        let encoded = encode_connect_request(123);

        assert_eq!(
            encoded.as_ref(),
            &[0, 0, 4, 23, 39, 16, 25, 128, 0, 0, 0, 0, 0, 0, 0, 123]
        );
    }

    #[test]
    fn decode_connect_response_validates_transaction_id() {
        let response = [0, 0, 0, 0, 0, 0, 0, 123, 0, 0, 0, 0, 0, 0, 3, 231];

        let decoded = decode_connect_response(&response, 123).unwrap();

        assert_eq!(decoded.connection_id, 999);
    }

    #[test]
    fn decode_connect_response_rejects_transaction_mismatch() {
        let response = [0, 0, 0, 0, 0, 0, 0, 124, 0, 0, 0, 0, 0, 0, 3, 231];

        let err = decode_connect_response(&response, 123).unwrap_err();

        assert!(matches!(
            err,
            TrackerError::TransactionIdMismatch {
                expected: 123,
                actual: 124
            }
        ));
    }

    #[test]
    fn decode_connect_response_rejects_short_packet() {
        let err = decode_connect_response(&[0; 15], 123).unwrap_err();

        assert!(matches!(
            err,
            TrackerError::InvalidUdpPacket {
                context: "connect response",
                actual: 15,
                minimum: 16
            }
        ));
    }

    #[test]
    fn encode_announce_request_matches_fixed_length() {
        let encoded =
            encode_announce_request(999, 123, &announce_request(), Ipv4Addr::UNSPECIFIED).unwrap();

        assert_eq!(encoded.len(), 98);
    }

    #[test]
    fn encode_announce_request_rejects_counter_overflow() {
        let mut request = announce_request();
        request.downloaded = u64::MAX;

        let err = encode_announce_request(999, 123, &request, Ipv4Addr::UNSPECIFIED).unwrap_err();

        assert!(matches!(
            err,
            TrackerError::InvalidIntegerRange {
                field: "downloaded"
            }
        ));
    }

    #[test]
    fn decode_announce_response_parses_ipv4_peers() {
        let response = [
            0, 0, 0, 1, 0, 0, 0, 123, 0, 0, 7, 8, 0, 0, 0, 2, 0, 0, 0, 3, 127, 0, 0, 1, 0x1a, 0xe1,
        ];

        let decoded = decode_announce_response(&response, 123).unwrap();

        assert_eq!(
            decoded.peers,
            vec![TrackerPeer {
                addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 6881),
                peer_id: None,
            }]
        );
    }

    #[test]
    fn decode_announce_response_rejects_invalid_peer_stride() {
        let response = [
            0, 0, 0, 1, 0, 0, 0, 123, 0, 0, 7, 8, 0, 0, 0, 2, 0, 0, 0, 3, 127,
        ];

        let err = decode_announce_response(&response, 123).unwrap_err();

        assert!(matches!(
            err,
            TrackerError::InvalidCompactPeerLength {
                actual: 1,
                stride: 6
            }
        ));
    }

    #[test]
    fn retry_delay_matches_bep15_schedule() {
        assert_eq!(retry_delay(8).unwrap(), Duration::from_secs(3840));
    }

    #[test]
    fn connection_cache_expires_after_one_minute() {
        let now = Instant::now();
        let cache = ConnectionIdCache::new(999, now);

        assert!(cache
            .connection_id_at(now + Duration::from_secs(60))
            .is_err());
    }

    #[test]
    fn encode_scrape_request_appends_info_hashes() {
        let request = ScrapeRequest {
            info_hashes: vec![InfoHashV1::new([7; 20]), InfoHashV1::new([8; 20])],
        };

        let encoded = encode_scrape_request(999, 123, &request);

        assert_eq!(encoded.len(), 56);
    }

    #[test]
    fn decode_scrape_response_maps_stats_to_requested_hashes() {
        let hashes = [InfoHashV1::new([7; 20]), InfoHashV1::new([8; 20])];
        let response = [
            0, 0, 0, 2, 0, 0, 0, 123, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 5, 0, 0, 0, 6, 0, 0, 0, 7,
            0, 0, 0, 8,
        ];

        let decoded = decode_scrape_response(&response, 123, &hashes).unwrap();

        assert_eq!(
            decoded.files,
            vec![
                (
                    InfoHashV1::new([7; 20]),
                    ScrapeStats {
                        complete: 3,
                        downloaded: 4,
                        incomplete: 5,
                    }
                ),
                (
                    InfoHashV1::new([8; 20]),
                    ScrapeStats {
                        complete: 6,
                        downloaded: 7,
                        incomplete: 8,
                    }
                ),
            ]
        );
    }

    #[tokio::test]
    #[ignore = "requires UDP socket permissions in the test environment"]
    async fn udp_tracker_client_connects_to_local_server() {
        let server = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_addr = server.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut buf = [0; 64];
            let (_len, peer) = server.recv_from(&mut buf).await.unwrap();
            server
                .send_to(&[0, 0, 0, 0, 0, 0, 0, 123, 0, 0, 0, 0, 0, 0, 3, 231], peer)
                .await
                .unwrap();
        });

        let client = UdpTrackerClient::new(Duration::from_secs(1));
        let response = client.connect_once(server_addr, 123).await.unwrap();
        task.await.unwrap();

        assert_eq!(response.connection_id, 999);
    }
}
