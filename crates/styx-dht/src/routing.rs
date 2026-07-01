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
    nodes: Vec<NodeRecord>,
}

impl RoutingTable {
    #[must_use]
    pub fn new(local_id: NodeId) -> Self {
        Self {
            local_id,
            buckets: vec![Bucket {
                prefix_len: 0,
                prefix: NodeId::new([0; 20]),
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
                return Ok(());
            }
            if self.buckets[bucket_index].nodes.len() < K_BUCKET_SIZE {
                self.buckets[bucket_index].nodes.push(NodeRecord {
                    id,
                    addr,
                    status: NodeStatus::Good,
                });
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
            nodes: Vec::new(),
        };
        let mut right = Bucket {
            prefix_len: next_len,
            prefix: right_prefix,
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
}

impl Bucket {
    fn contains(&self, id: NodeId) -> bool {
        (0..self.prefix_len).all(|bit| id.bit(bit) == self.prefix.bit(bit))
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
