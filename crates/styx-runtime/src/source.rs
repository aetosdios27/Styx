use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
};

use url::Url;

use crate::{RuntimeConfig, RuntimeError};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SourceKind {
    Peer,
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
    pub fn peer(address: SocketAddr) -> Self {
        Self {
            id: SourceId::new(0),
            kind: SourceKind::Peer,
            endpoint: SourceEndpoint::Peer(address),
        }
    }

    #[must_use]
    pub fn web_seed(url: Url) -> Self {
        Self {
            id: SourceId::new(0),
            kind: SourceKind::WebSeed,
            endpoint: SourceEndpoint::WebSeed(url),
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
        let mut next_id = 1_u64;

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
        entry.state = SourceState::Fresh;
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

    pub fn state(&self, source: SourceId) -> Result<SourceState, RuntimeError> {
        Ok(self.entry(source)?.state)
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
