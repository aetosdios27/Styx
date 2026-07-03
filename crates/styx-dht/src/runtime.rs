use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Instant;

use crate::{
    CompactNode, CompactPeer, DhtConfig, DhtError, DhtIdentityAction, DhtIdentityManager,
    DhtMessage, DhtNode, DhtQuery, DhtResponse, ExternalIp, InfoHash, Lookup, LookupKind, NodeAddr,
    NodeId, TokenManager, TransactionId, TransactionKind, TransactionTable, K_BUCKET_SIZE,
};
use bytes::Bytes;
use rand::{CryptoRng, RngCore};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DhtEvent {
    QueryResponded {
        target: NodeAddr,
    },
    ResponseMatched {
        source: NodeAddr,
        kind: TransactionKind,
    },
    UnsolicitedResponse {
        source: NodeAddr,
    },
    PeersDiscovered {
        info_hash: InfoHash,
        peers: Vec<CompactPeer>,
    },
    LookupExhausted {
        info_hash: InfoHash,
    },
    TransactionExpired {
        target: NodeAddr,
        kind: TransactionKind,
    },
    ExternalIpObserved {
        source: NodeAddr,
        ip: IpAddr,
    },
    ErrorReceived {
        source: NodeAddr,
        code: i64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeAction {
    pub response: Option<DhtMessage>,
    pub event: Option<DhtEvent>,
    pub outbound: Vec<(NodeAddr, DhtMessage)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DhtRuntime {
    node: DhtNode,
    config: DhtConfig,
    transactions: TransactionTable,
    get_peers_lookups: HashMap<InfoHash, Lookup>,
    announce_tokens: HashMap<(InfoHash, NodeAddr), Bytes>,
    identity: DhtIdentityManager,
    next_transaction: u16,
}

impl DhtRuntime {
    pub fn new(id: NodeId, tokens: TokenManager, config: DhtConfig) -> Result<Self, DhtError> {
        config.validate()?;
        Ok(Self {
            node: DhtNode::new(id, tokens),
            transactions: TransactionTable::with_capacity(config.max_transactions),
            config,
            get_peers_lookups: HashMap::new(),
            announce_tokens: HashMap::new(),
            identity: DhtIdentityManager::new(),
            next_transaction: 0,
        })
    }

    pub fn start_query(
        &mut self,
        target: NodeAddr,
        query: DhtQuery,
        now: Instant,
    ) -> Result<DhtMessage, DhtError> {
        let transaction_id = self.next_transaction_id()?;
        self.transactions.insert(
            transaction_id.clone(),
            target,
            TransactionKind::from_query(&query),
            now,
            self.config.query_timeout,
        )?;
        Ok(DhtMessage::Query {
            transaction_id,
            query,
        })
    }

    pub fn start_get_peers(
        &mut self,
        info_hash: InfoHash,
        now: Instant,
    ) -> Result<Vec<(NodeAddr, DhtMessage)>, DhtError> {
        let target = NodeId::new(*info_hash.as_bytes());
        let seeds = self
            .node
            .routing()
            .closest_nodes(target, K_BUCKET_SIZE)
            .into_iter()
            .map(|record| CompactNode {
                id: record.id,
                addr: record.addr,
            })
            .collect::<Vec<_>>();
        let mut lookup = Lookup::new(
            LookupKind::GetPeers { info_hash },
            self.config.lookup_alpha,
            seeds,
        );
        let batch = lookup.next_query_batch();
        self.get_peers_lookups.insert(info_hash, lookup);
        self.get_peers_queries(info_hash, batch, now)
    }

    pub fn start_bootstrap(
        &mut self,
        now: Instant,
    ) -> Result<Vec<(NodeAddr, DhtMessage)>, DhtError> {
        let id = self.node.id();
        self.config
            .bootstrap_nodes()
            .to_vec()
            .into_iter()
            .map(|addr| {
                let target = NodeAddr::new(addr);
                let message = self.start_query(target, DhtQuery::Ping { id }, now)?;
                Ok((target, message))
            })
            .collect()
    }

    pub fn start_announce_peer(
        &mut self,
        info_hash: InfoHash,
        port: u16,
        implied_port: bool,
        now: Instant,
    ) -> Result<Vec<(NodeAddr, DhtMessage)>, DhtError> {
        let mut targets = self
            .announce_tokens
            .iter()
            .filter(|((stored_hash, _), _)| *stored_hash == info_hash)
            .map(|((_, addr), token)| (*addr, token.clone()))
            .collect::<Vec<_>>();
        targets.sort_by_key(|(addr, _)| addr.socket_addr());
        let id = self.node.id();
        targets
            .into_iter()
            .map(|(target, token)| {
                let message = self.start_query(
                    target,
                    DhtQuery::AnnouncePeer {
                        id,
                        implied_port,
                        info_hash,
                        port,
                        token,
                    },
                    now,
                )?;
                Ok((target, message))
            })
            .collect()
    }

    pub fn handle_message(
        &mut self,
        message: DhtMessage,
        source: NodeAddr,
        now: Instant,
    ) -> Result<RuntimeAction, DhtError> {
        match message {
            DhtMessage::Query { .. } => {
                let response = self.node.handle_message(message, source)?;
                Ok(RuntimeAction {
                    response: Some(response),
                    event: Some(DhtEvent::QueryResponded { target: source }),
                    outbound: Vec::new(),
                })
            }
            DhtMessage::Response {
                transaction_id,
                response,
            } => match self
                .transactions
                .match_response(&transaction_id, source, now)
            {
                Ok(record) => {
                    self.remember_response_node(&response, source, now);
                    if let Some(action) =
                        self.handle_matched_get_peers(&record.kind, &response, source, now)?
                    {
                        return Ok(action);
                    }
                    if let Some(ip) = response.external_ip() {
                        return Ok(RuntimeAction {
                            response: None,
                            event: Some(DhtEvent::ExternalIpObserved { source, ip }),
                            outbound: Vec::new(),
                        });
                    }
                    Ok(RuntimeAction {
                        response: None,
                        event: Some(DhtEvent::ResponseMatched {
                            source,
                            kind: record.kind,
                        }),
                        outbound: Vec::new(),
                    })
                }
                Err(DhtError::UnexpectedTransaction) => Ok(RuntimeAction {
                    response: None,
                    event: Some(DhtEvent::UnsolicitedResponse { source }),
                    outbound: Vec::new(),
                }),
                Err(err) => Err(err),
            },
            DhtMessage::Error {
                transaction_id,
                error,
            } => {
                let _ = self
                    .transactions
                    .match_response(&transaction_id, source, now);
                Ok(RuntimeAction {
                    response: None,
                    event: Some(DhtEvent::ErrorReceived {
                        source,
                        code: error.code,
                    }),
                    outbound: Vec::new(),
                })
            }
        }
    }

    pub fn drain_timeouts(&mut self, now: Instant) -> Result<Vec<DhtEvent>, DhtError> {
        self.transactions
            .drain_expired(now)
            .into_iter()
            .map(|record| {
                if let Some(node) = self
                    .node
                    .routing()
                    .closest_nodes(self.node.id(), K_BUCKET_SIZE)
                    .into_iter()
                    .find(|node| node.addr == record.target)
                {
                    self.node.routing_mut().mark_failure(node.id)?;
                }
                Ok(DhtEvent::TransactionExpired {
                    target: record.target,
                    kind: record.kind,
                })
            })
            .collect()
    }

    pub fn observe_external_ip_for_identity<R>(
        &mut self,
        external_ip: ExternalIp,
        rng: &mut R,
    ) -> Result<Option<DhtIdentityAction>, DhtError>
    where
        R: RngCore + CryptoRng,
    {
        self.identity.observe_external_ip(external_ip, rng)
    }

    #[must_use]
    pub fn token_for(&self, info_hash: InfoHash, target: NodeAddr) -> Option<Bytes> {
        self.announce_tokens.get(&(info_hash, target)).cloned()
    }

    #[must_use]
    pub const fn node(&self) -> &DhtNode {
        &self.node
    }

    pub fn node_mut(&mut self) -> &mut DhtNode {
        &mut self.node
    }

    #[must_use]
    pub const fn transactions(&self) -> &TransactionTable {
        &self.transactions
    }

    fn next_transaction_id(&mut self) -> Result<TransactionId, DhtError> {
        self.next_transaction = self.next_transaction.wrapping_add(1);
        TransactionId::new(self.next_transaction.to_be_bytes().to_vec())
    }

    fn get_peers_queries(
        &mut self,
        info_hash: InfoHash,
        nodes: Vec<CompactNode>,
        now: Instant,
    ) -> Result<Vec<(NodeAddr, DhtMessage)>, DhtError> {
        let id = self.node.id();
        nodes
            .into_iter()
            .map(|node| {
                let message = self.start_query(
                    node.addr,
                    DhtQuery::GetPeers {
                        id,
                        info_hash,
                        want: Vec::new(),
                    },
                    now,
                )?;
                Ok((node.addr, message))
            })
            .collect()
    }

    fn handle_matched_get_peers(
        &mut self,
        kind: &TransactionKind,
        response: &DhtResponse,
        source: NodeAddr,
        now: Instant,
    ) -> Result<Option<RuntimeAction>, DhtError> {
        let TransactionKind::GetPeers { info_hash } = kind else {
            return Ok(None);
        };
        let DhtResponse::GetPeers {
            id,
            token,
            values,
            nodes,
            ..
        } = response
        else {
            return Ok(None);
        };
        self.announce_tokens
            .insert((*info_hash, source), token.clone());
        let Some(lookup) = self.get_peers_lookups.get_mut(info_hash) else {
            return Ok(None);
        };
        if values.is_empty() {
            lookup.on_nodes(*id, nodes.clone());
            let next = lookup.next_query_batch();
            let exhausted = next.is_empty() && lookup.is_complete();
            let outbound = self.get_peers_queries(*info_hash, next, now)?;
            Ok(Some(RuntimeAction {
                response: None,
                event: exhausted.then_some(DhtEvent::LookupExhausted {
                    info_hash: *info_hash,
                }),
                outbound,
            }))
        } else {
            lookup.on_peers(*id, values.clone());
            Ok(Some(RuntimeAction {
                response: None,
                event: Some(DhtEvent::PeersDiscovered {
                    info_hash: *info_hash,
                    peers: lookup.peers(),
                }),
                outbound: Vec::new(),
            }))
        }
    }

    fn remember_response_node(&mut self, response: &DhtResponse, source: NodeAddr, now: Instant) {
        let id = response.id();
        if self.node.routing_mut().insert(id, source).is_ok() {
            let _ = self.node.routing_mut().mark_seen(id, now);
        }
    }
}

impl DhtResponse {
    fn id(&self) -> NodeId {
        match self {
            Self::Ping { id }
            | Self::FindNode { id, .. }
            | Self::GetPeers { id, .. }
            | Self::AnnouncePeer { id } => *id,
        }
    }

    fn external_ip(&self) -> Option<IpAddr> {
        match self {
            Self::FindNode { external_ip, .. } | Self::GetPeers { external_ip, .. } => *external_ip,
            Self::Ping { .. } | Self::AnnouncePeer { .. } => None,
        }
    }
}

impl TransactionKind {
    fn from_query(query: &DhtQuery) -> Self {
        match query {
            DhtQuery::Ping { .. } => Self::Ping,
            DhtQuery::FindNode { target, .. } => Self::FindNode { target: *target },
            DhtQuery::GetPeers { info_hash, .. } => Self::GetPeers {
                info_hash: *info_hash,
            },
            DhtQuery::AnnouncePeer { info_hash, .. } => Self::AnnouncePeer {
                info_hash: *info_hash,
            },
        }
    }
}
