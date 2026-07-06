use std::net::SocketAddr;

use styx_proto::{InfoHashV1, PeerId};
use styx_tracker::{AnnounceEvent, AnnounceRequest, AnnounceResponse};

use crate::{TorrentPlan, TorrentSmokePlan};

#[must_use]
pub fn build_started_announce(
    plan: &TorrentSmokePlan,
    peer_id: PeerId,
    port: u16,
    numwant: u32,
) -> AnnounceRequest {
    AnnounceRequest {
        info_hash: plan.info_hash,
        peer_id,
        port,
        uploaded: 0,
        downloaded: 0,
        left: plan.left,
        event: Some(AnnounceEvent::Started),
        compact: true,
        numwant: Some(numwant),
        key: None,
    }
}

/// Build a periodic announce request for a running torrent.
///
/// Uses `None` event for regular re-announces after `Started`.
#[must_use]
pub fn build_announce_request(
    info_hash: InfoHashV1,
    peer_id: PeerId,
    port: u16,
    uploaded: u64,
    downloaded: u64,
    left: u64,
    numwant: u32,
) -> AnnounceRequest {
    AnnounceRequest {
        info_hash,
        peer_id,
        port,
        uploaded,
        downloaded,
        left,
        event: None,
        compact: true,
        numwant: Some(numwant),
        key: None,
    }
}

/// Convenience wrapper using `TorrentPlan` state.
#[must_use]
pub fn build_plan_announce_request(
    plan: &TorrentPlan,
    peer_id: PeerId,
    verified_bytes: u64,
    numwant: u32,
) -> AnnounceRequest {
    let port = 6881;
    let left = plan.total_size.saturating_sub(verified_bytes);
    build_announce_request(
        plan.info_hash,
        peer_id,
        port,
        0,      // uploaded
        0,      // downloaded
        left,
        numwant,
    )
}

/// Helper: pick unique, non-zero peers from a tracker response.
#[must_use]
pub fn select_peer_candidates(response: &AnnounceResponse, limit: usize) -> Vec<SocketAddr> {
    let mut peers = Vec::new();
    for peer in &response.peers {
        if peers.len() >= limit {
            break;
        }
        if peer.addr.port() == 0 || peer.addr.ip().is_unspecified() {
            continue;
        }
        if peers.contains(&peer.addr) {
            continue;
        }
        peers.push(peer.addr);
    }
    peers
}
