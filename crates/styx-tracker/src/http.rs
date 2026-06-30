use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;

use bytes::{Bytes, BytesMut};
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use styx_proto::{decode, BencodeValue, InfoHashV1, PeerId};
use url::Url;

use crate::{
    AnnounceEvent, AnnounceRequest, AnnounceResponse, ScrapeRequest, ScrapeResponse, ScrapeStats,
    TrackerError, TrackerPeer,
};

const IPV4_COMPACT_PEER_STRIDE: usize = 6;

/// Build an HTTP tracker announce URL.
///
/// # Errors
///
/// Returns [`TrackerError::InvalidUrl`] if the resulting URL cannot be
/// represented by the `url` crate.
pub fn build_announce_url(base: &Url, request: &AnnounceRequest) -> Result<Url, TrackerError> {
    let mut pairs = Vec::with_capacity(10);
    pairs.push(format!(
        "info_hash={}",
        percent_encode(request.info_hash.as_bytes(), NON_ALPHANUMERIC)
    ));
    pairs.push(format!(
        "peer_id={}",
        percent_encode(request.peer_id.as_bytes(), NON_ALPHANUMERIC)
    ));
    pairs.push(format!("port={}", request.port));
    pairs.push(format!("uploaded={}", request.uploaded));
    pairs.push(format!("downloaded={}", request.downloaded));
    pairs.push(format!("left={}", request.left));
    pairs.push(format!("compact={}", u8::from(request.compact)));
    if let Some(event) = request.event {
        pairs.push(format!("event={}", event.as_query_value()));
    }
    if let Some(numwant) = request.numwant {
        pairs.push(format!("numwant={numwant}"));
    }
    if let Some(key) = request.key {
        pairs.push(format!("key={key}"));
    }

    let mut url = base.clone();
    url.set_query(Some(&pairs.join("&")));
    Ok(url)
}

/// Build an HTTP tracker scrape URL using the BEP 3 announce-to-scrape path convention.
///
/// # Errors
///
/// Returns [`TrackerError::InvalidUrl`] when the base URL path does not contain
/// `announce`.
pub fn build_scrape_url(base: &Url, request: &ScrapeRequest) -> Result<Url, TrackerError> {
    let mut url = base.clone();
    let Some((prefix, suffix)) = url.path().rsplit_once("announce") else {
        return Err(TrackerError::InvalidUrl);
    };
    url.set_path(&format!("{prefix}scrape{suffix}"));

    let query = request
        .info_hashes
        .iter()
        .map(|info_hash| {
            format!(
                "info_hash={}",
                percent_encode(info_hash.as_bytes(), NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    url.set_query(Some(&query));
    Ok(url)
}

/// Async HTTP tracker client.
#[derive(Clone, Debug)]
pub struct HttpTrackerClient {
    client: reqwest::Client,
    max_response_bytes: usize,
}

impl HttpTrackerClient {
    /// Create an HTTP tracker client with a maximum response body size.
    #[must_use]
    pub fn new(max_response_bytes: usize) -> Self {
        Self {
            client: reqwest::Client::new(),
            max_response_bytes,
        }
    }

    /// Announce to an HTTP tracker.
    ///
    /// # Errors
    ///
    /// Returns [`TrackerError`] for URL construction, transport, body limit, or
    /// response parsing failures.
    pub async fn announce(
        &self,
        base: &Url,
        request: &AnnounceRequest,
    ) -> Result<AnnounceResponse, TrackerError> {
        let url = build_announce_url(base, request)?;
        let body = self.get_limited(url).await?;
        parse_announce_response(&body)
    }

    /// Scrape an HTTP tracker.
    ///
    /// # Errors
    ///
    /// Returns [`TrackerError`] for URL construction, transport, body limit, or
    /// response parsing failures.
    pub async fn scrape(
        &self,
        base: &Url,
        request: &ScrapeRequest,
    ) -> Result<ScrapeResponse, TrackerError> {
        let url = build_scrape_url(base, request)?;
        let body = self.get_limited(url).await?;
        parse_scrape_response(&body)
    }

    async fn get_limited(&self, url: Url) -> Result<Bytes, TrackerError> {
        let mut response = self.client.get(url).send().await?;
        let mut body = BytesMut::new();
        while let Some(chunk) = response.chunk().await? {
            let next_len = body.len() + chunk.len();
            if next_len > self.max_response_bytes {
                return Err(TrackerError::ResponseTooLarge {
                    actual: next_len,
                    max: self.max_response_bytes,
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body.freeze())
    }
}

/// Parse an HTTP tracker announce response.
///
/// # Errors
///
/// Returns [`TrackerError`] when the bencoded response is malformed or a
/// required announce field is missing.
pub fn parse_announce_response(input: &[u8]) -> Result<AnnounceResponse, TrackerError> {
    let value = decode(input)?;
    let dict = expect_dict(&value, "announce response")?;
    if let Some(reason) = optional_bytes(dict, b"failure reason", "failure reason")? {
        return Err(TrackerError::TrackerFailure { reason });
    }

    let interval = required_u32(dict, b"interval", "interval")?;
    let min_interval = optional_u32(dict, b"min interval", "min interval")?;
    let tracker_id = optional_bytes(dict, b"tracker id", "tracker id")?;
    let seeders = optional_u32(dict, b"complete", "complete")?;
    let leechers = optional_u32(dict, b"incomplete", "incomplete")?;
    let warning_message = optional_bytes(dict, b"warning message", "warning message")?;
    let peers = parse_peers(required_value(dict, b"peers", "peers")?)?;

    Ok(AnnounceResponse {
        interval,
        min_interval,
        tracker_id,
        seeders,
        leechers,
        peers,
        warning_message,
    })
}

/// Parse an HTTP tracker scrape response.
///
/// # Errors
///
/// Returns [`TrackerError`] when the bencoded response is malformed or scrape
/// file stats are missing required fields.
pub fn parse_scrape_response(input: &[u8]) -> Result<ScrapeResponse, TrackerError> {
    let value = decode(input)?;
    let dict = expect_dict(&value, "scrape response")?;
    if let Some(reason) = optional_bytes(dict, b"failure reason", "failure reason")? {
        return Err(TrackerError::TrackerFailure { reason });
    }

    let files_value = required_value(dict, b"files", "files")?;
    let files_dict = expect_dict(files_value, "files")?;
    let mut files = Vec::with_capacity(files_dict.len());
    for (raw_info_hash, stats_value) in files_dict {
        if raw_info_hash.len() != 20 {
            return Err(TrackerError::InvalidInfoHashLength { field: "files" });
        }
        let mut bytes = [0; 20];
        bytes.copy_from_slice(raw_info_hash);
        let stats_dict = expect_dict(stats_value, "files entry")?;
        files.push((
            InfoHashV1::new(bytes),
            ScrapeStats {
                complete: required_u32(stats_dict, b"complete", "complete")?,
                downloaded: required_u32(stats_dict, b"downloaded", "downloaded")?,
                incomplete: required_u32(stats_dict, b"incomplete", "incomplete")?,
            },
        ));
    }

    Ok(ScrapeResponse { files })
}

impl AnnounceEvent {
    fn as_query_value(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Stopped => "stopped",
            Self::Completed => "completed",
        }
    }
}

fn parse_peers(value: &BencodeValue) -> Result<Vec<TrackerPeer>, TrackerError> {
    match value {
        BencodeValue::Bytes(bytes) => parse_compact_ipv4_peers(bytes),
        BencodeValue::List(peers) => peers.iter().map(parse_dictionary_peer).collect(),
        _ => Err(TrackerError::WrongType { field: "peers" }),
    }
}

fn parse_compact_ipv4_peers(bytes: &[u8]) -> Result<Vec<TrackerPeer>, TrackerError> {
    if !bytes.len().is_multiple_of(IPV4_COMPACT_PEER_STRIDE) {
        return Err(TrackerError::InvalidCompactPeerLength {
            actual: bytes.len(),
            stride: IPV4_COMPACT_PEER_STRIDE,
        });
    }

    bytes
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

fn parse_dictionary_peer(value: &BencodeValue) -> Result<TrackerPeer, TrackerError> {
    let dict = expect_dict(value, "peer")?;
    let ip_bytes = required_bytes(dict, b"ip", "ip")?;
    let ip_text = std::str::from_utf8(&ip_bytes)
        .map_err(|_| TrackerError::InvalidPeerAddress { field: "ip" })?;
    let ip =
        IpAddr::from_str(ip_text).map_err(|_| TrackerError::InvalidPeerAddress { field: "ip" })?;
    let port = required_u16(dict, b"port", "port")?;
    let peer_id = optional_bytes(dict, b"peer id", "peer id")?
        .map(|bytes| {
            if bytes.len() != 20 {
                return Err(TrackerError::WrongType { field: "peer id" });
            }
            let mut peer_id = [0; 20];
            peer_id.copy_from_slice(&bytes);
            Ok(PeerId::new(peer_id))
        })
        .transpose()?;

    Ok(TrackerPeer {
        addr: SocketAddr::new(ip, port),
        peer_id,
    })
}

fn expect_dict<'a>(
    value: &'a BencodeValue,
    field: &'static str,
) -> Result<&'a std::collections::BTreeMap<Vec<u8>, BencodeValue>, TrackerError> {
    match value {
        BencodeValue::Dict(dict) => Ok(dict),
        _ => Err(TrackerError::WrongType { field }),
    }
}

fn required_value<'a>(
    dict: &'a std::collections::BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    field: &'static str,
) -> Result<&'a BencodeValue, TrackerError> {
    dict.get(key).ok_or(TrackerError::MissingField { field })
}

fn required_bytes(
    dict: &std::collections::BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    field: &'static str,
) -> Result<Bytes, TrackerError> {
    match required_value(dict, key, field)? {
        BencodeValue::Bytes(bytes) => Ok(bytes.clone()),
        _ => Err(TrackerError::WrongType { field }),
    }
}

fn optional_bytes(
    dict: &std::collections::BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    field: &'static str,
) -> Result<Option<Bytes>, TrackerError> {
    let Some(value) = dict.get(key) else {
        return Ok(None);
    };
    match value {
        BencodeValue::Bytes(bytes) => Ok(Some(bytes.clone())),
        _ => Err(TrackerError::WrongType { field }),
    }
}

fn required_u32(
    dict: &std::collections::BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    field: &'static str,
) -> Result<u32, TrackerError> {
    match required_value(dict, key, field)? {
        BencodeValue::Integer(value) => {
            u32::try_from(*value).map_err(|_| TrackerError::InvalidIntegerRange { field })
        }
        _ => Err(TrackerError::WrongType { field }),
    }
}

fn optional_u32(
    dict: &std::collections::BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    field: &'static str,
) -> Result<Option<u32>, TrackerError> {
    let Some(value) = dict.get(key) else {
        return Ok(None);
    };
    match value {
        BencodeValue::Integer(value) => u32::try_from(*value)
            .map(Some)
            .map_err(|_| TrackerError::InvalidIntegerRange { field }),
        _ => Err(TrackerError::WrongType { field }),
    }
}

fn required_u16(
    dict: &std::collections::BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    field: &'static str,
) -> Result<u16, TrackerError> {
    match required_value(dict, key, field)? {
        BencodeValue::Integer(value) => {
            u16::try_from(*value).map_err(|_| TrackerError::InvalidIntegerRange { field })
        }
        _ => Err(TrackerError::WrongType { field }),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use bytes::Bytes;
    use styx_proto::{InfoHashV1, PeerId};
    use url::Url;

    use crate::{
        build_announce_url, build_scrape_url, parse_announce_response, parse_scrape_response,
        AnnounceEvent, AnnounceRequest, ScrapeRequest, ScrapeStats, TrackerError, TrackerPeer,
    };

    fn announce_request() -> AnnounceRequest {
        let mut info_hash = [b'a'; 20];
        info_hash[0] = 0x00;
        info_hash[1] = 0xff;
        info_hash[2] = b'/';
        let mut peer_id = [b'b'; 20];
        peer_id[0] = 0xff;

        AnnounceRequest {
            info_hash: InfoHashV1::new(info_hash),
            peer_id: PeerId::new(peer_id),
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
    fn build_announce_url_percent_encodes_raw_info_hash_bytes() {
        let base = Url::parse("https://tracker.example/announce").unwrap();

        let url = build_announce_url(&base, &announce_request()).unwrap();

        assert!(url.as_str().contains("info_hash=%00%FF%2F"));
    }

    #[test]
    fn build_announce_url_percent_encodes_raw_peer_id_bytes() {
        let base = Url::parse("https://tracker.example/announce").unwrap();

        let url = build_announce_url(&base, &announce_request()).unwrap();

        assert!(url.as_str().contains("peer_id=%FF"));
    }

    #[test]
    fn parse_announce_response_returns_tracker_failure() {
        let err = parse_announce_response(b"d14:failure reason9:not founde").unwrap_err();

        assert!(
            matches!(err, TrackerError::TrackerFailure { reason } if reason == Bytes::from_static(b"not found"))
        );
    }

    #[test]
    fn parse_announce_response_accepts_compact_ipv4_peers() {
        let response = parse_announce_response(
            b"d8:intervali1800e5:peers12:\x7f\x00\x00\x01\x1a\xe1\x0a\x00\x00\x02\x1a\xe2e",
        )
        .unwrap();

        assert_eq!(
            response.peers,
            vec![
                TrackerPeer {
                    addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 6881),
                    peer_id: None,
                },
                TrackerPeer {
                    addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)), 6882),
                    peer_id: None,
                },
            ]
        );
    }

    #[test]
    fn parse_announce_response_rejects_invalid_compact_peer_length() {
        let err = parse_announce_response(b"d8:intervali1800e5:peers7:abcdefge").unwrap_err();

        assert!(matches!(
            err,
            TrackerError::InvalidCompactPeerLength {
                actual: 7,
                stride: 6
            }
        ));
    }

    #[test]
    fn parse_announce_response_accepts_dictionary_peers() {
        let response = parse_announce_response(
            b"d8:intervali1800e5:peersld2:ip9:127.0.0.17:peer id20:ABCDEFGHIJKLMNOPQRST4:porti6881eeee",
        )
        .unwrap();

        assert_eq!(
            response.peers,
            vec![TrackerPeer {
                addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 6881),
                peer_id: Some(PeerId::new(*b"ABCDEFGHIJKLMNOPQRST")),
            }]
        );
    }

    #[test]
    fn parse_announce_response_rejects_missing_interval() {
        let err = parse_announce_response(b"d5:peers0:e").unwrap_err();

        assert!(matches!(
            err,
            TrackerError::MissingField { field: "interval" }
        ));
    }

    #[test]
    fn parse_scrape_response_accepts_stats_by_info_hash() {
        let mut input = b"d5:filesd20:".to_vec();
        input.extend_from_slice(&[7; 20]);
        input.extend_from_slice(b"d8:completei3e10:downloadedi4e10:incompletei5eeee");

        let response = parse_scrape_response(&input).unwrap();

        assert_eq!(
            response.files,
            vec![(
                InfoHashV1::new([7; 20]),
                ScrapeStats {
                    complete: 3,
                    downloaded: 4,
                    incomplete: 5,
                }
            )]
        );
    }

    #[test]
    fn build_scrape_url_replaces_announce_path_and_encodes_hashes() {
        let base = Url::parse("https://tracker.example/path/announce").unwrap();
        let request = ScrapeRequest {
            info_hashes: vec![InfoHashV1::new([0xff; 20])],
        };

        let url = build_scrape_url(&base, &request).unwrap();

        assert!(url
            .as_str()
            .starts_with("https://tracker.example/path/scrape?"));
    }
}
