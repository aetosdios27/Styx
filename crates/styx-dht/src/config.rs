use std::net::SocketAddr;
use std::time::Duration;

use crate::DhtError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DhtConfig {
    bootstrap_nodes: Vec<SocketAddr>,
    pub query_timeout: Duration,
    pub bucket_refresh_interval: Duration,
    pub lookup_alpha: usize,
    pub max_transactions: usize,
    pub max_peers_per_info_hash: usize,
}

impl Default for DhtConfig {
    fn default() -> Self {
        Self {
            bootstrap_nodes: Vec::new(),
            query_timeout: Duration::from_secs(15),
            bucket_refresh_interval: Duration::from_secs(15 * 60),
            lookup_alpha: 3,
            max_transactions: 1024,
            max_peers_per_info_hash: 128,
        }
    }
}

impl DhtConfig {
    pub fn validate(&self) -> Result<(), DhtError> {
        if self.query_timeout.is_zero() {
            return Err(DhtError::InvalidConfig("query_timeout"));
        }
        if self.bucket_refresh_interval.is_zero() {
            return Err(DhtError::InvalidConfig("bucket_refresh_interval"));
        }
        if self.lookup_alpha == 0 {
            return Err(DhtError::InvalidConfig("lookup_alpha"));
        }
        if self.max_transactions == 0 {
            return Err(DhtError::InvalidConfig("max_transactions"));
        }
        if self.max_peers_per_info_hash == 0 {
            return Err(DhtError::InvalidConfig("max_peers_per_info_hash"));
        }
        Ok(())
    }

    pub fn add_bootstrap_node(&mut self, addr: SocketAddr) {
        if !self.bootstrap_nodes.contains(&addr) {
            self.bootstrap_nodes.push(addr);
        }
    }

    #[must_use]
    pub fn bootstrap_nodes(&self) -> &[SocketAddr] {
        &self.bootstrap_nodes
    }
}
