//! BitTorrent tracker protocol primitives and clients.
//!
//! `styx-tracker` owns tracker announce/scrape protocol correctness for HTTP,
//! UDP, and BEP 12 multitracker policy. It does not manage peer connections or
//! torrent lifecycle state.

pub mod error;
pub mod http;
pub mod multitracker;
pub mod types;
pub mod udp;

pub use error::TrackerError;
pub use http::{
    build_announce_url, build_scrape_url, parse_announce_response, parse_scrape_response,
    HttpTrackerClient,
};
pub use multitracker::{TrackerTier, TrackerTierList};
pub use types::{
    AnnounceEvent, AnnounceRequest, AnnounceResponse, ScrapeRequest, ScrapeResponse, ScrapeStats,
    TrackerPeer,
};
pub use udp::{
    decode_announce_response, decode_connect_response, decode_scrape_response,
    encode_announce_request, encode_connect_request, encode_scrape_request, retry_delay,
    ConnectionIdCache, UdpAction, UdpAnnounceResponse, UdpConnectResponse, UdpTrackerClient,
};
