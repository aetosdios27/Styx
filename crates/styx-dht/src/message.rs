use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use bytes::Bytes;
use styx_proto::{decode, encode, BencodeValue};

use crate::{CompactNode, CompactPeer, DhtError, InfoHash, NodeId, TransactionId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DhtMessage {
    Query {
        transaction_id: TransactionId,
        query: DhtQuery,
    },
    Response {
        transaction_id: TransactionId,
        response: DhtResponse,
    },
    Error {
        transaction_id: TransactionId,
        error: KrpcError,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DhtQuery {
    Ping {
        id: NodeId,
    },
    FindNode {
        id: NodeId,
        target: NodeId,
        want: Vec<AddressFamily>,
    },
    GetPeers {
        id: NodeId,
        info_hash: InfoHash,
        want: Vec<AddressFamily>,
    },
    AnnouncePeer {
        id: NodeId,
        implied_port: bool,
        info_hash: InfoHash,
        port: u16,
        token: Bytes,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DhtResponse {
    Ping {
        id: NodeId,
    },
    FindNode {
        id: NodeId,
        nodes: Vec<CompactNode>,
        nodes6: Vec<CompactNode>,
        external_ip: Option<IpAddr>,
    },
    GetPeers {
        id: NodeId,
        token: Bytes,
        values: Vec<CompactPeer>,
        nodes: Vec<CompactNode>,
        nodes6: Vec<CompactNode>,
        external_ip: Option<IpAddr>,
    },
    AnnouncePeer {
        id: NodeId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KrpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
}

impl DhtMessage {
    pub fn encode(&self) -> Result<Vec<u8>, DhtError> {
        Ok(encode(&self.to_bencode()?))
    }

    pub fn decode(input: &[u8]) -> Result<Self, DhtError> {
        Self::from_bencode(decode(input)?)
    }

    fn to_bencode(&self) -> Result<BencodeValue, DhtError> {
        let mut root = BTreeMap::new();
        match self {
            Self::Query {
                transaction_id,
                query,
            } => {
                root.insert(b"t".to_vec(), bytes(transaction_id.as_bytes()));
                root.insert(b"y".to_vec(), bytes(b"q"));
                root.insert(b"q".to_vec(), bytes(query.name().as_bytes()));
                root.insert(b"a".to_vec(), BencodeValue::Dict(query.arguments()));
            }
            Self::Response {
                transaction_id,
                response,
            } => {
                root.insert(b"t".to_vec(), bytes(transaction_id.as_bytes()));
                root.insert(b"y".to_vec(), bytes(b"r"));
                root.insert(b"r".to_vec(), BencodeValue::Dict(response.fields()?));
            }
            Self::Error {
                transaction_id,
                error,
            } => {
                root.insert(b"t".to_vec(), bytes(transaction_id.as_bytes()));
                root.insert(b"y".to_vec(), bytes(b"e"));
                root.insert(
                    b"e".to_vec(),
                    BencodeValue::List(vec![
                        BencodeValue::Integer(error.code),
                        bytes(error.message.as_bytes()),
                    ]),
                );
            }
        }
        Ok(BencodeValue::Dict(root))
    }

    fn from_bencode(value: BencodeValue) -> Result<Self, DhtError> {
        let BencodeValue::Dict(root) = value else {
            return Err(DhtError::InvalidMessage(
                "top-level KRPC value must be a dictionary",
            ));
        };
        let transaction_id = transaction_id(field(&root, b"t", "t")?)?;
        let kind = bytes_field(&root, b"y", "y")?;
        match kind {
            b"q" => {
                let query_name = bytes_field(&root, b"q", "q")?;
                let args = dict_field(&root, b"a", "a")?;
                Ok(Self::Query {
                    transaction_id,
                    query: DhtQuery::from_parts(query_name, args)?,
                })
            }
            b"r" => {
                let fields = dict_field(&root, b"r", "r")?;
                Ok(Self::Response {
                    transaction_id,
                    response: DhtResponse::from_fields(fields)?,
                })
            }
            b"e" => Ok(Self::Error {
                transaction_id,
                error: krpc_error(field(&root, b"e", "e")?)?,
            }),
            _ => Err(DhtError::InvalidMessage("unknown KRPC message kind")),
        }
    }
}

impl DhtQuery {
    fn name(&self) -> &'static str {
        match self {
            Self::Ping { .. } => "ping",
            Self::FindNode { .. } => "find_node",
            Self::GetPeers { .. } => "get_peers",
            Self::AnnouncePeer { .. } => "announce_peer",
        }
    }

    fn arguments(&self) -> BTreeMap<Vec<u8>, BencodeValue> {
        let mut args = BTreeMap::new();
        match self {
            Self::Ping { id } => {
                insert_id(&mut args, *id);
            }
            Self::FindNode { id, target, want } => {
                insert_id(&mut args, *id);
                args.insert(b"target".to_vec(), bytes(target.as_bytes()));
                insert_want(&mut args, want);
            }
            Self::GetPeers {
                id,
                info_hash,
                want,
            } => {
                insert_id(&mut args, *id);
                args.insert(b"info_hash".to_vec(), bytes(info_hash.as_bytes()));
                insert_want(&mut args, want);
            }
            Self::AnnouncePeer {
                id,
                implied_port,
                info_hash,
                port,
                token,
            } => {
                insert_id(&mut args, *id);
                args.insert(
                    b"implied_port".to_vec(),
                    BencodeValue::Integer(i64::from(*implied_port)),
                );
                args.insert(b"info_hash".to_vec(), bytes(info_hash.as_bytes()));
                args.insert(b"port".to_vec(), BencodeValue::Integer(i64::from(*port)));
                args.insert(b"token".to_vec(), bytes(token));
            }
        }
        args
    }

    fn from_parts(name: &[u8], args: &BTreeMap<Vec<u8>, BencodeValue>) -> Result<Self, DhtError> {
        match name {
            b"ping" => Ok(Self::Ping {
                id: node_id_field(args, b"id", "id")?,
            }),
            b"find_node" => Ok(Self::FindNode {
                id: node_id_field(args, b"id", "id")?,
                target: node_id_field(args, b"target", "target")?,
                want: optional_want(args)?,
            }),
            b"get_peers" => Ok(Self::GetPeers {
                id: node_id_field(args, b"id", "id")?,
                info_hash: info_hash_field(args, b"info_hash", "info_hash")?,
                want: optional_want(args)?,
            }),
            b"announce_peer" => Ok(Self::AnnouncePeer {
                id: node_id_field(args, b"id", "id")?,
                implied_port: integer_field_optional(args, b"implied_port")?.unwrap_or(0) != 0,
                info_hash: info_hash_field(args, b"info_hash", "info_hash")?,
                port: port_field(args, b"port", "port")?,
                token: Bytes::copy_from_slice(bytes_field(args, b"token", "token")?),
            }),
            _ => Err(DhtError::InvalidMessage("unknown DHT query name")),
        }
    }
}

impl DhtResponse {
    fn fields(&self) -> Result<BTreeMap<Vec<u8>, BencodeValue>, DhtError> {
        let mut fields = BTreeMap::new();
        match self {
            Self::Ping { id } | Self::AnnouncePeer { id } => {
                insert_id(&mut fields, *id);
            }
            Self::FindNode {
                id,
                nodes,
                nodes6,
                external_ip,
            } => {
                insert_id(&mut fields, *id);
                insert_nodes(&mut fields, nodes, nodes6)?;
                insert_external_ip(&mut fields, *external_ip);
            }
            Self::GetPeers {
                id,
                token,
                values,
                nodes,
                nodes6,
                external_ip,
            } => {
                insert_id(&mut fields, *id);
                fields.insert(b"token".to_vec(), bytes(token));
                insert_external_ip(&mut fields, *external_ip);
                if !values.is_empty() {
                    fields.insert(
                        b"values".to_vec(),
                        BencodeValue::List(values.iter().map(encode_peer_value).collect::<Result<
                            Vec<_>,
                            DhtError,
                        >>(
                        )?),
                    );
                }
                insert_nodes(&mut fields, nodes, nodes6)?;
            }
        }
        Ok(fields)
    }

    fn from_fields(fields: &BTreeMap<Vec<u8>, BencodeValue>) -> Result<Self, DhtError> {
        let id = node_id_field(fields, b"id", "id")?;
        if let Some(token) = optional_bytes_field(fields, b"token")? {
            let values = optional_peer_values(fields)?;
            let nodes = optional_nodes_field(fields)?.unwrap_or_default();
            let nodes6 = optional_nodes6_field(fields)?.unwrap_or_default();
            let external_ip = optional_external_ip(fields)?;
            return Ok(Self::GetPeers {
                id,
                token: Bytes::copy_from_slice(token),
                values,
                nodes,
                nodes6,
                external_ip,
            });
        }
        let nodes = optional_nodes_field(fields)?;
        let nodes6 = optional_nodes6_field(fields)?;
        if nodes.is_some() || nodes6.is_some() {
            return Ok(Self::FindNode {
                id,
                nodes: nodes.unwrap_or_default(),
                nodes6: nodes6.unwrap_or_default(),
                external_ip: optional_external_ip(fields)?,
            });
        }
        Ok(Self::Ping { id })
    }
}

fn insert_id(fields: &mut BTreeMap<Vec<u8>, BencodeValue>, id: NodeId) {
    fields.insert(b"id".to_vec(), bytes(id.as_bytes()));
}

fn insert_want(fields: &mut BTreeMap<Vec<u8>, BencodeValue>, want: &[AddressFamily]) {
    if want.is_empty() {
        return;
    }
    fields.insert(
        b"want".to_vec(),
        BencodeValue::List(
            want.iter()
                .map(|family| bytes(family.as_bytes()))
                .collect::<Vec<_>>(),
        ),
    );
}

fn insert_nodes(
    fields: &mut BTreeMap<Vec<u8>, BencodeValue>,
    nodes: &[CompactNode],
    nodes6: &[CompactNode],
) -> Result<(), DhtError> {
    if !nodes.is_empty() {
        fields.insert(
            b"nodes".to_vec(),
            BencodeValue::Bytes(CompactNode::encode_many_ipv4(nodes)?),
        );
    }
    if !nodes6.is_empty() {
        fields.insert(
            b"nodes6".to_vec(),
            BencodeValue::Bytes(CompactNode::encode_many_ipv6(nodes6)?),
        );
    }
    Ok(())
}

fn insert_external_ip(fields: &mut BTreeMap<Vec<u8>, BencodeValue>, external_ip: Option<IpAddr>) {
    let Some(ip) = external_ip else {
        return;
    };
    let bytes = match ip {
        IpAddr::V4(ip) => Bytes::copy_from_slice(&ip.octets()),
        IpAddr::V6(ip) => Bytes::copy_from_slice(&ip.octets()),
    };
    fields.insert(b"ip".to_vec(), BencodeValue::Bytes(bytes));
}

fn field<'a>(
    dict: &'a BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    name: &'static str,
) -> Result<&'a BencodeValue, DhtError> {
    dict.get(key).ok_or(DhtError::MissingField(name))
}

fn dict_field<'a>(
    dict: &'a BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    name: &'static str,
) -> Result<&'a BTreeMap<Vec<u8>, BencodeValue>, DhtError> {
    match field(dict, key, name)? {
        BencodeValue::Dict(value) => Ok(value),
        _ => Err(DhtError::InvalidField(name)),
    }
}

fn bytes_field<'a>(
    dict: &'a BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    name: &'static str,
) -> Result<&'a [u8], DhtError> {
    match field(dict, key, name)? {
        BencodeValue::Bytes(value) => Ok(value),
        _ => Err(DhtError::InvalidField(name)),
    }
}

fn optional_bytes_field<'a>(
    dict: &'a BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
) -> Result<Option<&'a [u8]>, DhtError> {
    let Some(value) = dict.get(key) else {
        return Ok(None);
    };
    match value {
        BencodeValue::Bytes(bytes) => Ok(Some(bytes)),
        _ => Err(DhtError::InvalidField("bytes")),
    }
}

fn node_id_field(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    name: &'static str,
) -> Result<NodeId, DhtError> {
    NodeId::try_from(bytes_field(dict, key, name)?)
}

fn info_hash_field(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    name: &'static str,
) -> Result<InfoHash, DhtError> {
    InfoHash::try_from(bytes_field(dict, key, name)?)
}

fn integer_field_optional(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
) -> Result<Option<i64>, DhtError> {
    let Some(value) = dict.get(key) else {
        return Ok(None);
    };
    match value {
        BencodeValue::Integer(value) => Ok(Some(*value)),
        _ => Err(DhtError::InvalidField("integer")),
    }
}

fn port_field(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    name: &'static str,
) -> Result<u16, DhtError> {
    let value = match field(dict, key, name)? {
        BencodeValue::Integer(value) => *value,
        _ => return Err(DhtError::InvalidField(name)),
    };
    u16::try_from(value).map_err(|_| DhtError::InvalidField(name))
}

fn optional_nodes_field(
    fields: &BTreeMap<Vec<u8>, BencodeValue>,
) -> Result<Option<Vec<CompactNode>>, DhtError> {
    let Some(nodes) = optional_bytes_field(fields, b"nodes")? else {
        return Ok(None);
    };
    Ok(Some(CompactNode::decode_many_ipv4(nodes)?))
}

fn optional_nodes6_field(
    fields: &BTreeMap<Vec<u8>, BencodeValue>,
) -> Result<Option<Vec<CompactNode>>, DhtError> {
    let Some(nodes) = optional_bytes_field(fields, b"nodes6")? else {
        return Ok(None);
    };
    Ok(Some(CompactNode::decode_many_ipv6(nodes)?))
}

fn optional_peer_values(
    fields: &BTreeMap<Vec<u8>, BencodeValue>,
) -> Result<Vec<CompactPeer>, DhtError> {
    let Some(values) = fields.get(b"values".as_slice()) else {
        return Ok(Vec::new());
    };
    let BencodeValue::List(values) = values else {
        return Err(DhtError::InvalidField("values"));
    };
    values
        .iter()
        .map(|value| match value {
            BencodeValue::Bytes(bytes) if bytes.len() == 6 => CompactPeer::decode_ipv4(bytes),
            BencodeValue::Bytes(bytes) if bytes.len() == 18 => CompactPeer::decode_ipv6(bytes),
            BencodeValue::Bytes(bytes) => Err(DhtError::InvalidLength {
                expected: 6,
                actual: bytes.len(),
            }),
            _ => Err(DhtError::InvalidField("values")),
        })
        .collect()
}

fn optional_want(dict: &BTreeMap<Vec<u8>, BencodeValue>) -> Result<Vec<AddressFamily>, DhtError> {
    let Some(value) = dict.get(b"want".as_slice()) else {
        return Ok(Vec::new());
    };
    let BencodeValue::List(values) = value else {
        return Err(DhtError::InvalidField("want"));
    };
    values
        .iter()
        .map(|value| match value {
            BencodeValue::Bytes(bytes) => AddressFamily::try_from(bytes.as_ref()),
            _ => Err(DhtError::InvalidField("want")),
        })
        .collect()
}

fn optional_external_ip(
    fields: &BTreeMap<Vec<u8>, BencodeValue>,
) -> Result<Option<IpAddr>, DhtError> {
    let Some(bytes) = optional_bytes_field(fields, b"ip")? else {
        return Ok(None);
    };
    match bytes.len() {
        4 => Ok(Some(IpAddr::V4(Ipv4Addr::new(
            bytes[0], bytes[1], bytes[2], bytes[3],
        )))),
        16 => Ok(Some(IpAddr::V6(Ipv6Addr::from(
            <[u8; 16]>::try_from(bytes).map_err(|_| DhtError::InvalidField("ip"))?,
        )))),
        _ => Err(DhtError::InvalidField("ip")),
    }
}

fn encode_peer_value(peer: &CompactPeer) -> Result<BencodeValue, DhtError> {
    match peer.socket_addr() {
        std::net::SocketAddr::V4(_) => peer
            .encode_ipv4()
            .map(|peer| BencodeValue::Bytes(Bytes::copy_from_slice(&peer))),
        std::net::SocketAddr::V6(_) => peer
            .encode_ipv6()
            .map(|peer| BencodeValue::Bytes(Bytes::copy_from_slice(&peer))),
    }
}

fn transaction_id(value: &BencodeValue) -> Result<TransactionId, DhtError> {
    let BencodeValue::Bytes(bytes) = value else {
        return Err(DhtError::InvalidField("t"));
    };
    TransactionId::new(bytes.to_vec())
}

fn krpc_error(value: &BencodeValue) -> Result<KrpcError, DhtError> {
    let BencodeValue::List(values) = value else {
        return Err(DhtError::InvalidField("e"));
    };
    if values.len() != 2 {
        return Err(DhtError::InvalidField("e"));
    }
    let BencodeValue::Integer(code) = values[0] else {
        return Err(DhtError::InvalidField("e"));
    };
    let BencodeValue::Bytes(message) = &values[1] else {
        return Err(DhtError::InvalidField("e"));
    };
    let message = std::str::from_utf8(message)
        .map_err(|_| DhtError::InvalidField("e"))?
        .to_owned();
    Ok(KrpcError { code, message })
}

fn bytes(value: &[u8]) -> BencodeValue {
    BencodeValue::Bytes(Bytes::copy_from_slice(value))
}

impl AddressFamily {
    fn as_bytes(self) -> &'static [u8] {
        match self {
            Self::Ipv4 => b"n4",
            Self::Ipv6 => b"n6",
        }
    }
}

impl TryFrom<&[u8]> for AddressFamily {
    type Error = DhtError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        match value {
            b"n4" => Ok(Self::Ipv4),
            b"n6" => Ok(Self::Ipv6),
            _ => Err(DhtError::InvalidField("want")),
        }
    }
}
