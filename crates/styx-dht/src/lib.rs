//! BEP 5 DHT protocol, routing, and discovery primitives for Styx.

mod error;
mod lookup;
mod message;
mod node;
mod routing;
mod security;
mod socket;
mod store;
mod token;
mod types;

pub use error::DhtError;
pub use lookup::{Lookup, LookupKind};
pub use message::{DhtMessage, DhtQuery, DhtResponse, KrpcError};
pub use node::DhtNode;
pub use routing::{NodeRecord, NodeStatus, RoutingTable, K_BUCKET_SIZE};
pub use security::{generate_bep42_ipv4_id, is_bep42_ipv4_id};
pub use socket::{DhtSocket, SocketEvent};
pub use store::PeerStore;
pub use token::TokenManager;
pub use types::{
    CompactNode, CompactPeer, InfoHash, NodeAddr, NodeDistance, NodeId, TransactionId,
};
