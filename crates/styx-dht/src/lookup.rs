use std::collections::{HashMap, HashSet};

use crate::{CompactNode, CompactPeer, InfoHash, NodeId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LookupKind {
    FindNode { target: NodeId },
    GetPeers { info_hash: InfoHash },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lookup {
    kind: LookupKind,
    alpha: usize,
    candidates: HashMap<NodeId, CompactNode>,
    queried: HashSet<NodeId>,
    in_flight: HashSet<NodeId>,
    peers: Vec<CompactPeer>,
}

impl Lookup {
    #[must_use]
    pub fn new(kind: LookupKind, alpha: usize, seeds: Vec<CompactNode>) -> Self {
        let candidates = seeds.into_iter().map(|node| (node.id, node)).collect();
        Self {
            kind,
            alpha,
            candidates,
            queried: HashSet::new(),
            in_flight: HashSet::new(),
            peers: Vec::new(),
        }
    }

    pub fn next_query_batch(&mut self) -> Vec<CompactNode> {
        if self.is_complete() {
            return Vec::new();
        }
        let remaining = self.alpha.saturating_sub(self.in_flight.len());
        let mut nodes = self
            .candidates
            .values()
            .copied()
            .filter(|node| !self.queried.contains(&node.id) && !self.in_flight.contains(&node.id))
            .collect::<Vec<_>>();
        nodes.sort_by_key(|node| self.target().distance(&node.id));
        nodes.truncate(remaining);
        for node in &nodes {
            self.in_flight.insert(node.id);
        }
        nodes
    }

    pub fn on_nodes(&mut self, responder: NodeId, nodes: Vec<CompactNode>) {
        self.queried.insert(responder);
        self.in_flight.remove(&responder);
        for node in nodes {
            self.candidates.entry(node.id).or_insert(node);
        }
    }

    pub fn on_peers(&mut self, responder: NodeId, peers: Vec<CompactPeer>) {
        self.queried.insert(responder);
        self.in_flight.remove(&responder);
        for peer in peers {
            if !self.peers.contains(&peer) {
                self.peers.push(peer);
            }
        }
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        (!self.peers.is_empty() && matches!(self.kind, LookupKind::GetPeers { .. }))
            || (self.in_flight.is_empty()
                && self
                    .candidates
                    .keys()
                    .all(|node| self.queried.contains(node)))
    }

    #[must_use]
    pub fn peers(&self) -> Vec<CompactPeer> {
        self.peers.clone()
    }

    fn target(&self) -> NodeId {
        match self.kind {
            LookupKind::FindNode { target } => target,
            LookupKind::GetPeers { info_hash } => NodeId::new(*info_hash.as_bytes()),
        }
    }
}
