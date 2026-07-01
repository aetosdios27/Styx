use std::net::SocketAddr;

use crate::{
    CompactNode, CompactPeer, DhtError, DhtMessage, DhtQuery, DhtResponse, InfoHash, NodeAddr,
    NodeId, PeerStore, RoutingTable, TokenManager, TransactionId, K_BUCKET_SIZE,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DhtNode {
    id: NodeId,
    routing: RoutingTable,
    tokens: TokenManager,
    peers: PeerStore,
}

impl DhtNode {
    #[must_use]
    pub fn new(id: NodeId, tokens: TokenManager) -> Self {
        Self {
            id,
            routing: RoutingTable::new(id),
            tokens,
            peers: PeerStore::with_capacity(128),
        }
    }

    pub fn handle_message(
        &mut self,
        message: DhtMessage,
        source: NodeAddr,
    ) -> Result<DhtMessage, DhtError> {
        let DhtMessage::Query {
            transaction_id,
            query,
        } = message
        else {
            return Err(DhtError::InvalidMessage(
                "node handlers accept queries only",
            ));
        };
        self.remember_query_sender(&query, source)?;
        match query {
            DhtQuery::Ping { .. } => {
                Ok(response(transaction_id, DhtResponse::Ping { id: self.id }))
            }
            DhtQuery::FindNode { target, .. } => {
                let nodes = self
                    .routing
                    .closest_nodes(target, K_BUCKET_SIZE)
                    .into_iter()
                    .filter(|record| record.addr != source)
                    .map(|record| CompactNode {
                        id: record.id,
                        addr: record.addr,
                    })
                    .collect();
                Ok(response(
                    transaction_id,
                    DhtResponse::FindNode {
                        id: self.id,
                        nodes,
                        nodes6: Vec::new(),
                        external_ip: None,
                    },
                ))
            }
            DhtQuery::GetPeers { info_hash, .. } => {
                let values = self.peers.peers(info_hash);
                let nodes = if values.is_empty() {
                    self.routing
                        .closest_nodes(NodeId::new(*info_hash.as_bytes()), K_BUCKET_SIZE)
                        .into_iter()
                        .filter(|record| record.addr != source)
                        .map(|record| CompactNode {
                            id: record.id,
                            addr: record.addr,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                Ok(response(
                    transaction_id,
                    DhtResponse::GetPeers {
                        id: self.id,
                        token: self.tokens.issue(source.socket_addr().ip()),
                        values,
                        nodes,
                        nodes6: Vec::new(),
                        external_ip: None,
                    },
                ))
            }
            DhtQuery::AnnouncePeer {
                implied_port,
                info_hash,
                port,
                token,
                ..
            } => {
                if !self.tokens.validate(source.socket_addr().ip(), &token) {
                    return Err(DhtError::InvalidToken);
                }
                let peer_addr = if implied_port {
                    source.socket_addr()
                } else {
                    with_port(source.socket_addr(), port)
                };
                self.peers
                    .announce(info_hash, CompactPeer::new(peer_addr))?;
                Ok(response(
                    transaction_id,
                    DhtResponse::AnnouncePeer { id: self.id },
                ))
            }
        }
    }

    pub fn announce_local_peer(
        &mut self,
        info_hash: InfoHash,
        peer: CompactPeer,
    ) -> Result<(), DhtError> {
        self.peers.announce(info_hash, peer)
    }

    #[must_use]
    pub fn peers(&self, info_hash: InfoHash) -> Vec<CompactPeer> {
        self.peers.peers(info_hash)
    }

    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.id
    }

    #[must_use]
    pub const fn routing(&self) -> &RoutingTable {
        &self.routing
    }

    pub fn routing_mut(&mut self) -> &mut RoutingTable {
        &mut self.routing
    }

    fn remember_query_sender(
        &mut self,
        query: &DhtQuery,
        source: NodeAddr,
    ) -> Result<(), DhtError> {
        let id = match query {
            DhtQuery::Ping { id }
            | DhtQuery::FindNode { id, .. }
            | DhtQuery::GetPeers { id, .. }
            | DhtQuery::AnnouncePeer { id, .. } => *id,
        };
        match self.routing.insert(id, source) {
            Ok(()) | Err(DhtError::BucketFull) => Ok(()),
            Err(err) => Err(err),
        }
    }
}

fn response(transaction_id: TransactionId, response: DhtResponse) -> DhtMessage {
    DhtMessage::Response {
        transaction_id,
        response,
    }
}

fn with_port(addr: SocketAddr, port: u16) -> SocketAddr {
    SocketAddr::new(addr.ip(), port)
}
