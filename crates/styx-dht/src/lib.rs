//! BEP 5 DHT protocol, routing, and discovery primitives for Styx.

mod cache;
mod config;
mod error;
mod lookup;
mod message;
mod node;
mod routing;
mod runtime;
mod security;
mod socket;
mod store;
mod token;
mod transaction;
mod types;

pub use cache::{RoutingCache, RoutingCacheEntry};
pub use config::DhtConfig;
pub use error::DhtError;
pub use lookup::{Lookup, LookupKind};
pub use message::{AddressFamily, DhtMessage, DhtQuery, DhtResponse, KrpcError};
pub use node::DhtNode;
pub use routing::{NodeRecord, NodeStatus, RoutingTable, K_BUCKET_SIZE};
pub use runtime::{DhtEvent, DhtRuntime, RuntimeAction};
pub use security::{generate_bep42_ipv4_id, is_bep42_ipv4_id, SourceRateLimiter};
pub use socket::{DhtSocket, DhtSocketRuntime, SocketEvent};
pub use store::PeerStore;
pub use token::TokenManager;
pub use transaction::{TransactionKind, TransactionRecord, TransactionTable};
pub use types::{
    CompactNode, CompactPeer, InfoHash, NodeAddr, NodeDistance, NodeId, TransactionId,
};
