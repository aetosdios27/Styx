use bytes::Bytes;
use styx_proto::{InfoHashV1, PeerId};

/// Tracker announce lifecycle event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnnounceEvent {
    /// Client started participating in a torrent.
    Started,
    /// Client stopped participating in a torrent.
    Stopped,
    /// Client completed the download.
    Completed,
}

/// Parameters sent to tracker announce endpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnounceRequest {
    /// Torrent v1 info hash.
    pub info_hash: InfoHashV1,
    /// Local peer id for this torrent session.
    pub peer_id: PeerId,
    /// Listening port.
    pub port: u16,
    /// Total uploaded bytes.
    pub uploaded: u64,
    /// Total downloaded bytes.
    pub downloaded: u64,
    /// Bytes left to download.
    pub left: u64,
    /// Optional announce event.
    pub event: Option<AnnounceEvent>,
    /// Request compact peer response.
    pub compact: bool,
    /// Optional requested peer count.
    pub numwant: Option<u32>,
    /// Optional client key used by some trackers.
    pub key: Option<u32>,
}

/// Parsed tracker announce response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnounceResponse {
    /// Recommended seconds between announces.
    pub interval: u32,
    /// Optional minimum interval before announcing again.
    pub min_interval: Option<u32>,
    /// Optional tracker id to echo in later announces.
    pub tracker_id: Option<Bytes>,
    /// Optional seeder count.
    pub seeders: Option<u32>,
    /// Optional leecher count.
    pub leechers: Option<u32>,
    /// Peers returned by the tracker.
    pub peers: Vec<TrackerPeer>,
    /// Optional non-fatal tracker warning.
    pub warning_message: Option<Bytes>,
}

/// One peer endpoint returned by a tracker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackerPeer {
    /// Peer socket address.
    pub addr: std::net::SocketAddr,
    /// Optional peer id. Compact peer lists do not include this.
    pub peer_id: Option<PeerId>,
}

/// Parameters sent to tracker scrape endpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScrapeRequest {
    /// Info hashes to scrape.
    pub info_hashes: Vec<InfoHashV1>,
}

/// Parsed tracker scrape response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScrapeResponse {
    /// Per-info-hash scrape stats, in response or requested order.
    pub files: Vec<(InfoHashV1, ScrapeStats)>,
}

/// Aggregate scrape stats for one torrent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScrapeStats {
    /// Number of seeders.
    pub complete: u32,
    /// Number of completed downloads.
    pub downloaded: u32,
    /// Number of leechers.
    pub incomplete: u32,
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use bytes::Bytes;
    use styx_proto::{InfoHashV1, PeerId};

    use super::*;

    #[test]
    fn announce_request_can_represent_started_compact_request() {
        let request = AnnounceRequest {
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
        };

        assert_eq!(request.event, Some(AnnounceEvent::Started));
    }

    #[test]
    fn announce_response_can_hold_peer_without_peer_id() {
        let response = AnnounceResponse {
            interval: 1800,
            min_interval: Some(900),
            tracker_id: Some(Bytes::from_static(b"tracker-1")),
            seeders: Some(10),
            leechers: Some(2),
            peers: vec![TrackerPeer {
                addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 6881),
                peer_id: None,
            }],
            warning_message: None,
        };

        assert_eq!(response.peers[0].peer_id, None);
    }

    #[test]
    fn scrape_response_preserves_info_hash_stats() {
        let info_hash = InfoHashV1::new([7; 20]);
        let response = ScrapeResponse {
            files: vec![(
                info_hash,
                ScrapeStats {
                    complete: 3,
                    downloaded: 4,
                    incomplete: 5,
                },
            )],
        };

        assert_eq!(response.files[0].0, info_hash);
    }
}
