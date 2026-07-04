use std::net::SocketAddr;

use styx_proto::PeerId;
use styx_tracker::{AnnounceEvent, AnnounceRequest, AnnounceResponse};

use crate::TorrentSmokePlan;

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
