use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    net::SocketAddr,
};

use url::Url;

use crate::{RuntimeConfig, RuntimeError};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SourceKind {
    Peer,
    DhtPeer,
    PexPeer,
    WebSeed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCandidate {
    pub id: SourceId,
    pub kind: SourceKind,
    pub endpoint: SourceEndpoint,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SourceEndpoint {
    Peer(SocketAddr),
    WebSeed(Url),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceState {
    Fresh,
    Active,
    CoolingDown,
    Quarantined,
    Exhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceFailure {
    Timeout,
    Refused,
    HttpStatus(u16),
    ProtocolViolation,
    CorruptData,
}

#[derive(Clone, Debug)]
pub struct SourceTable {
    retry_limit: usize,
    max_sources: usize,
    next_id: u64,
    entries: BTreeMap<SourceId, SourceEntry>,
}

#[derive(Clone, Debug)]
struct SourceEntry {
    candidate: SourceCandidate,
    state: SourceState,
    failures: usize,
}

impl SourceCandidate {
    #[must_use]
    pub fn peer(id: SourceId, address: SocketAddr) -> Self {
        Self {
            id,
            kind: SourceKind::Peer,
            endpoint: SourceEndpoint::Peer(address),
        }
    }

    #[must_use]
    pub fn web_seed(id: SourceId, url: Url) -> Self {
        Self {
            id,
            kind: SourceKind::WebSeed,
            endpoint: SourceEndpoint::WebSeed(url),
        }
    }

    #[must_use]
    pub fn dht_peer(id: SourceId, address: SocketAddr) -> Self {
        Self {
            id,
            kind: SourceKind::DhtPeer,
            endpoint: SourceEndpoint::Peer(address),
        }
    }
}

impl SourceId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl SourceTable {
    pub fn from_candidates(
        candidates: Vec<SourceCandidate>,
        config: &RuntimeConfig,
    ) -> Result<Self, RuntimeError> {
        config.clone().validate()?;
        let mut seen = BTreeSet::new();
        let mut entries = BTreeMap::new();
        let mut next_id: u64 = 1;

        for mut candidate in candidates {
            if !seen.insert(candidate.endpoint.clone()) {
                continue;
            }
            if entries.len() == config.limits.max_sources_per_torrent {
                break;
            }
            let id = SourceId::new(next_id);
            next_id += 1;
            candidate.id = id;
            entries.insert(
                id,
                SourceEntry {
                    candidate,
                    state: SourceState::Fresh,
                    failures: 0,
                },
            );
        }

        Ok(Self {
            retry_limit: config.limits.source_retry_limit,
            max_sources: config.limits.max_sources_per_torrent,
            next_id,
            entries,
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn by_kind(&self, kind: SourceKind) -> impl Iterator<Item = &SourceCandidate> {
        self.entries
            .values()
            .filter(move |entry| entry.candidate.kind == kind)
            .map(|entry| &entry.candidate)
    }

    #[must_use]
    pub fn next_candidates(&self, limit: usize) -> Vec<SourceCandidate> {
        self.entries
            .values()
            .filter(|entry| matches!(entry.state, SourceState::Fresh | SourceState::CoolingDown))
            .take(limit)
            .map(|entry| entry.candidate.clone())
            .collect()
    }

    pub fn record_success(&mut self, source: SourceId) -> Result<(), RuntimeError> {
        let entry = self.entry_mut(source)?;
        entry.state = SourceState::Active;
        entry.failures = 0;
        Ok(())
    }

    pub fn record_failure(
        &mut self,
        source: SourceId,
        failure: SourceFailure,
    ) -> Result<(), RuntimeError> {
        let retry_limit = self.retry_limit;
        let entry = self.entry_mut(source)?;
        match failure {
            SourceFailure::CorruptData | SourceFailure::ProtocolViolation => {
                entry.state = SourceState::Quarantined;
            }
            SourceFailure::Timeout | SourceFailure::Refused | SourceFailure::HttpStatus(_) => {
                entry.failures += 1;
                entry.state = if entry.failures >= retry_limit {
                    SourceState::Exhausted
                } else {
                    SourceState::CoolingDown
                };
            }
        }
        Ok(())
    }

    pub fn add_candidate(
        &mut self,
        endpoint: SourceEndpoint,
        kind: SourceKind,
    ) -> Result<SourceId, RuntimeError> {
        if self.entries.len() >= self.max_sources {
            return Err(RuntimeError::SourceTableFull);
        }
        if self
            .entries
            .values()
            .any(|e| e.candidate.endpoint == endpoint)
        {
            return Err(RuntimeError::DuplicateSource);
        }
        let id = SourceId::new(self.next_id);
        self.next_id += 1;
        let candidate = SourceCandidate { id, kind, endpoint };
        self.entries.insert(
            id,
            SourceEntry {
                candidate,
                state: SourceState::Fresh,
                failures: 0,
            },
        );
        Ok(id)
    }

    pub fn add_dht_peer(&mut self, address: SocketAddr) -> Result<SourceId, RuntimeError> {
        self.add_candidate(SourceEndpoint::Peer(address), SourceKind::DhtPeer)
    }

    pub fn add_pex_peer(&mut self, address: SocketAddr) -> Result<SourceId, RuntimeError> {
        self.add_candidate(SourceEndpoint::Peer(address), SourceKind::PexPeer)
    }

    pub fn state(&self, source: SourceId) -> Result<SourceState, RuntimeError> {
        Ok(self.entry(source)?.state)
    }

    pub fn id_for_endpoint(&self, endpoint: &SourceEndpoint) -> Option<SourceId> {
        self.entries
            .iter()
            .find_map(|(id, entry)| (entry.candidate.endpoint == *endpoint).then_some(*id))
    }

    fn entry(&self, source: SourceId) -> Result<&SourceEntry, RuntimeError> {
        self.entries.get(&source).ok_or(RuntimeError::SourceFailed {
            source_id: format!("source:{}", source.get()),
            scope: crate::FailureScope::TorrentGlobal,
            retry: crate::RetryClass::Terminal,
            reason: "unknown source".to_owned(),
        })
    }

    fn entry_mut(&mut self, source: SourceId) -> Result<&mut SourceEntry, RuntimeError> {
        self.entries
            .get_mut(&source)
            .ok_or(RuntimeError::SourceFailed {
                source_id: format!("source:{}", source.get()),
                scope: crate::FailureScope::TorrentGlobal,
                retry: crate::RetryClass::Terminal,
                reason: "unknown source".to_owned(),
            })
    }
}

/// Tracks block-level corruption at the (piece, block) granularity per peer.
#[derive(Debug, Default)]
pub struct BlockCorruptionTracker {
    /// Track which peers sent bad data for which (piece, block)
    bad_blocks: HashMap<(u32, u32), Vec<SocketAddr>>,
    /// Peers that have been quarantined for block-level corruption
    quarantined_peers: HashSet<SocketAddr>,
    /// Number of block failures before a peer is quarantined
    max_failures: usize,
}

impl BlockCorruptionTracker {
    #[must_use]
    pub fn new(max_failures: usize) -> Self {
        Self {
            max_failures,
            ..Default::default()
        }
    }

    /// Record that a peer sent corrupt data for a specific block.
    /// Returns `true` if the peer should now be quarantined.
    pub fn record_failure(&mut self, piece: u32, block: u32, peer: SocketAddr) -> bool {
        self.bad_blocks
            .entry((piece, block))
            .or_default()
            .push(peer);
        let total = self
            .bad_blocks
            .values()
            .filter(|v| v.contains(&peer))
            .count();
        if total >= self.max_failures {
            self.quarantined_peers.insert(peer);
            return true;
        }
        false
    }

    /// Whether a peer has been quarantined for block-level corruption.
    #[must_use]
    pub fn is_quarantined(&self, peer: &SocketAddr) -> bool {
        self.quarantined_peers.contains(peer)
    }

    /// Peers that sent bad data for a specific block.
    #[must_use]
    pub fn bad_peers_for_block(&self, piece: u32, block: u32) -> &[SocketAddr] {
        self.bad_blocks
            .get(&(piece, block))
            .map(|v| v.as_slice())
            .unwrap_or_default()
    }

    /// Peers that sent bad data for a specific block (owned copy).
    #[must_use]
    pub fn untrusted_peers_for_block(&self, piece: u32, block: u32) -> Vec<SocketAddr> {
        self.bad_blocks
            .get(&(piece, block))
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use crate::{RuntimeConfig, RuntimeLimits};

    use super::*;

    fn create_table(candidates: Vec<SourceCandidate>) -> SourceTable {
        let config = RuntimeConfig {
            limits: RuntimeLimits {
                max_sources_per_torrent: 10,
                source_retry_limit: 3,
                ..RuntimeLimits::default()
            },
            ..RuntimeConfig::default()
        };
        SourceTable::from_candidates(candidates, &config).unwrap()
    }

    #[test]
    fn record_success_sets_state_to_active_not_fresh() {
        let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 6881);
        let peer = SourceCandidate::peer(SourceId::new(0), addr);
        let mut table = create_table(vec![peer]);

        let sid = SourceId::new(1);
        table.record_success(sid).unwrap();

        assert_eq!(table.state(sid).unwrap(), SourceState::Active);
    }

    #[test]
    fn next_candidates_does_not_return_successful_sources() {
        let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 6881);
        let peer = SourceCandidate::peer(SourceId::new(0), addr);
        let mut table = create_table(vec![peer]);

        let sid = SourceId::new(1);
        table.record_success(sid).unwrap();

        let candidates = table.next_candidates(10);
        assert!(
            candidates.is_empty(),
            "successful source should not be returned as a candidate"
        );
    }

    #[test]
    fn next_candidates_returns_fresh_and_cooling_down_sources() {
        let addr1 = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 6881);
        let addr2 = SocketAddr::new(Ipv4Addr::new(10, 0, 0, 1).into(), 6881);
        let mut table = create_table(vec![
            SourceCandidate::peer(SourceId::new(0), addr1),
            SourceCandidate::peer(SourceId::new(0), addr2),
        ]);

        let sid = SourceId::new(1);
        table.record_success(sid).unwrap();

        let candidates = table.next_candidates(10);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].endpoint, SourceEndpoint::Peer(addr2));
    }

    #[test]
    fn block_corruption_tracker_quarantines_peer_after_threshold() {
        let mut tracker = BlockCorruptionTracker::new(2);
        let peer_a = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 6881);
        let peer_b = SocketAddr::new(Ipv4Addr::new(10, 0, 0, 1).into(), 6881);

        assert!(!tracker.record_failure(0, 1, peer_a));
        assert!(!tracker.is_quarantined(&peer_a));
        assert!(tracker.bad_peers_for_block(0, 1).contains(&peer_a));

        assert!(tracker.record_failure(0, 2, peer_a));
        assert!(tracker.is_quarantined(&peer_a));
        assert!(!tracker.is_quarantined(&peer_b));

        let untrusted = tracker.untrusted_peers_for_block(0, 1);
        assert!(untrusted.contains(&peer_a));
        assert!(tracker.bad_peers_for_block(0, 2).contains(&peer_a));
    }
}
