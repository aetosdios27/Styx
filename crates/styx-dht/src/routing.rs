use std::time::{Duration, Instant};

use crate::{DhtError, NodeAddr, NodeId};

pub const K_BUCKET_SIZE: usize = 8;
const NODE_ID_BITS: usize = 160;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeStatus {
    Good,
    Questionable,
    Bad,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeRecord {
    pub id: NodeId,
    pub addr: NodeAddr,
    pub status: NodeStatus,
    pub last_seen: Instant,
    pub last_queried: Option<Instant>,
    pub failures: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutingTable {
    local_id: NodeId,
    buckets: Vec<Bucket>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Bucket {
    prefix_len: usize,
    prefix: NodeId,
    last_changed: Instant,
    nodes: Vec<NodeRecord>,
}

impl RoutingTable {
    #[must_use]
    pub fn new(local_id: NodeId) -> Self {
        let now = Instant::now();
        Self {
            local_id,
            buckets: vec![Bucket {
                prefix_len: 0,
                prefix: NodeId::new([0; 20]),
                last_changed: now,
                nodes: Vec::new(),
            }],
        }
    }

    pub fn insert(&mut self, id: NodeId, addr: NodeAddr) -> Result<(), DhtError> {
        if id == self.local_id {
            return Ok(());
        }
        loop {
            let bucket_index = self.bucket_index(id);
            if let Some(record) = self.buckets[bucket_index]
                .nodes
                .iter_mut()
                .find(|record| record.id == id)
            {
                record.addr = addr;
                record.status = NodeStatus::Good;
                record.last_seen = Instant::now();
                record.failures = 0;
                return Ok(());
            }
            if self.buckets[bucket_index].nodes.len() < K_BUCKET_SIZE {
                let now = Instant::now();
                self.buckets[bucket_index].nodes.push(NodeRecord {
                    id,
                    addr,
                    status: NodeStatus::Good,
                    last_seen: now,
                    last_queried: None,
                    failures: 0,
                });
                self.buckets[bucket_index].last_changed = now;
                return Ok(());
            }
            if !self.buckets[bucket_index].contains(self.local_id)
                || self.buckets[bucket_index].prefix_len >= NODE_ID_BITS
            {
                return Err(DhtError::BucketFull);
            }
            self.split_bucket(bucket_index);
        }
    }

    pub fn mark_questionable(&mut self, id: NodeId) -> Result<(), DhtError> {
        let bucket_index = self.bucket_index(id);
        let Some(record) = self.buckets[bucket_index]
            .nodes
            .iter_mut()
            .find(|record| record.id == id)
        else {
            return Err(DhtError::UnknownNode);
        };
        record.status = NodeStatus::Questionable;
        Ok(())
    }

    pub fn mark_seen(&mut self, id: NodeId, now: Instant) -> Result<(), DhtError> {
        let record = self.node_mut(id)?;
        record.status = NodeStatus::Good;
        record.last_seen = now;
        record.failures = 0;
        Ok(())
    }

    pub fn mark_queried(&mut self, id: NodeId, now: Instant) -> Result<(), DhtError> {
        let record = self.node_mut(id)?;
        record.last_queried = Some(now);
        Ok(())
    }

    pub fn mark_failure(&mut self, id: NodeId) -> Result<(), DhtError> {
        let record = self.node_mut(id)?;
        record.failures = record.failures.saturating_add(1);
        record.status = if record.failures >= 3 {
            NodeStatus::Bad
        } else {
            NodeStatus::Questionable
        };
        Ok(())
    }

    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<NodeRecord> {
        let bucket_index = self.bucket_index(id);
        self.buckets[bucket_index]
            .nodes
            .iter()
            .find(|record| record.id == id)
            .copied()
    }

    #[must_use]
    pub fn stale_refresh_target(&self, now: Instant, refresh_after: Duration) -> Option<NodeId> {
        self.buckets
            .iter()
            .find(|bucket| now.duration_since(bucket.last_changed) >= refresh_after)
            .map(Bucket::refresh_target)
            .filter(|target| *target != self.local_id)
    }

    #[must_use]
    pub fn closest_nodes(&self, target: NodeId, limit: usize) -> Vec<NodeRecord> {
        let mut nodes = self
            .buckets
            .iter()
            .flat_map(|bucket| bucket.nodes.iter().copied())
            .filter(|record| record.status != NodeStatus::Bad)
            .collect::<Vec<_>>();
        nodes.sort_by_key(|record| target.distance(&record.id));
        nodes.truncate(limit);
        nodes
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.buckets.iter().map(|bucket| bucket.nodes.len()).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    fn bucket_index(&self, id: NodeId) -> usize {
        self.buckets
            .iter()
            .position(|bucket| bucket.contains(id))
            .unwrap_or(0)
    }

    fn split_bucket(&mut self, bucket_index: usize) {
        let bucket = self.buckets.remove(bucket_index);
        let next_len = bucket.prefix_len + 1;
        let left_prefix = bucket.prefix.with_prefix_bit(next_len - 1, false);
        let right_prefix = bucket.prefix.with_prefix_bit(next_len - 1, true);
        let mut left = Bucket {
            prefix_len: next_len,
            prefix: left_prefix,
            last_changed: Instant::now(),
            nodes: Vec::new(),
        };
        let mut right = Bucket {
            prefix_len: next_len,
            prefix: right_prefix,
            last_changed: Instant::now(),
            nodes: Vec::new(),
        };
        for node in bucket.nodes {
            if left.contains(node.id) {
                left.nodes.push(node);
            } else {
                right.nodes.push(node);
            }
        }
        self.buckets.push(left);
        self.buckets.push(right);
    }

    fn node_mut(&mut self, id: NodeId) -> Result<&mut NodeRecord, DhtError> {
        let bucket_index = self.bucket_index(id);
        self.buckets[bucket_index]
            .nodes
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or(DhtError::UnknownNode)
    }
}

impl Bucket {
    fn contains(&self, id: NodeId) -> bool {
        (0..self.prefix_len).all(|bit| id.bit(bit) == self.prefix.bit(bit))
    }

    fn refresh_target(&self) -> NodeId {
        if self.prefix_len == 0 {
            return NodeId::new([0x80; 20]);
        }
        self.prefix.with_prefix_bit(self.prefix_len - 1, true)
    }
}

impl NodeId {
    fn bit(&self, bit_index: usize) -> bool {
        let byte = bit_index / 8;
        let bit = 7 - (bit_index % 8);
        self.as_bytes()[byte] & (1 << bit) != 0
    }

    fn with_prefix_bit(self, bit_index: usize, value: bool) -> Self {
        let mut bytes = *self.as_bytes();
        let byte = bit_index / 8;
        let bit = 7 - (bit_index % 8);
        if value {
            bytes[byte] |= 1 << bit;
        } else {
            bytes[byte] &= !(1 << bit);
        }
        Self::new(bytes)
    }
}
