use std::collections::HashMap;

use crate::{CompactPeer, DhtError, InfoHash};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerStore {
    peers: HashMap<InfoHash, Vec<CompactPeer>>,
    capacity_per_hash: usize,
}

impl PeerStore {
    #[must_use]
    pub fn with_capacity(capacity_per_hash: usize) -> Self {
        Self {
            peers: HashMap::new(),
            capacity_per_hash,
        }
    }

    pub fn announce(&mut self, info_hash: InfoHash, peer: CompactPeer) -> Result<(), DhtError> {
        let peers = self.peers.entry(info_hash).or_default();
        if peers.contains(&peer) {
            return Ok(());
        }
        if peers.len() >= self.capacity_per_hash {
            return Err(DhtError::PeerStoreFull);
        }
        peers.push(peer);
        Ok(())
    }

    #[must_use]
    pub fn peers(&self, info_hash: InfoHash) -> Vec<CompactPeer> {
        self.peers.get(&info_hash).cloned().unwrap_or_default()
    }
}
