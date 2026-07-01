use std::time::{Duration, Instant};

use crate::{NodeAddr, NodeId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoutingCacheEntry {
    pub id: NodeId,
    pub addr: NodeAddr,
    pub last_seen: Instant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutingCache {
    entries: Vec<RoutingCacheEntry>,
}

impl RoutingCache {
    #[must_use]
    pub fn from_entries(local_id: NodeId, entries: Vec<RoutingCacheEntry>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .filter(|entry| entry.id != local_id)
                .collect(),
        }
    }

    #[must_use]
    pub const fn local_id(&self) -> Option<NodeId> {
        None
    }

    #[must_use]
    pub fn entries(&self, now: Instant, ttl: Duration) -> Vec<RoutingCacheEntry> {
        self.entries
            .iter()
            .copied()
            .filter(|entry| now.duration_since(entry.last_seen) <= ttl)
            .collect()
    }
}
