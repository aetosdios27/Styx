use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::{DhtError, InfoHash, NodeAddr, NodeId, TransactionId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionKind {
    Ping,
    FindNode { target: NodeId },
    GetPeers { info_hash: InfoHash },
    AnnouncePeer { info_hash: InfoHash },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionRecord {
    pub transaction_id: TransactionId,
    pub target: NodeAddr,
    pub kind: TransactionKind,
    pub started_at: Instant,
    pub deadline: Instant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionTable {
    capacity: usize,
    records: HashMap<(TransactionId, NodeAddr), TransactionRecord>,
}

impl TransactionTable {
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            records: HashMap::with_capacity(capacity),
        }
    }

    pub fn insert(
        &mut self,
        transaction_id: TransactionId,
        target: NodeAddr,
        kind: TransactionKind,
        now: Instant,
        timeout: Duration,
    ) -> Result<(), DhtError> {
        if self.records.len() >= self.capacity {
            return Err(DhtError::TransactionTableFull);
        }
        let record = TransactionRecord {
            transaction_id: transaction_id.clone(),
            target,
            kind,
            started_at: now,
            deadline: now + timeout,
        };
        self.records.insert((transaction_id, target), record);
        Ok(())
    }

    pub fn match_response(
        &mut self,
        transaction_id: &TransactionId,
        source: NodeAddr,
        now: Instant,
    ) -> Result<TransactionRecord, DhtError> {
        self.drain_expired(now);
        self.records
            .remove(&(transaction_id.clone(), source))
            .ok_or(DhtError::UnexpectedTransaction)
    }

    pub fn drain_expired(&mut self, now: Instant) -> Vec<TransactionRecord> {
        let expired = self
            .records
            .iter()
            .filter_map(|(key, record)| (record.deadline <= now).then_some(key.clone()))
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|key| self.records.remove(&key))
            .collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}
