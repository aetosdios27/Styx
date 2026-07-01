use std::convert::TryFrom;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bytes::Bytes;

use crate::DhtError;

pub const NODE_ID_LEN: usize = 20;
pub const INFO_HASH_LEN: usize = 20;
pub const IPV4_COMPACT_NODE_LEN: usize = 26;
pub const IPV4_COMPACT_PEER_LEN: usize = 6;
pub const MAX_TRANSACTION_ID_LEN: usize = 4;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId([u8; NODE_ID_LEN]);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InfoHash([u8; INFO_HASH_LEN]);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TransactionId(Bytes);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NodeAddr(SocketAddr);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CompactNode {
    pub id: NodeId,
    pub addr: NodeAddr,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CompactPeer {
    addr: SocketAddr,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeDistance([u8; NODE_ID_LEN]);

impl NodeId {
    #[must_use]
    pub const fn new(value: [u8; NODE_ID_LEN]) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; NODE_ID_LEN] {
        &self.0
    }

    #[must_use]
    pub fn distance(&self, other: &Self) -> NodeDistance {
        let mut distance = [0_u8; NODE_ID_LEN];
        for ((output, left), right) in distance.iter_mut().zip(self.0).zip(other.0) {
            *output = left ^ right;
        }
        NodeDistance(distance)
    }
}

impl TryFrom<&[u8]> for NodeId {
    type Error = DhtError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let bytes = <[u8; NODE_ID_LEN]>::try_from(value).map_err(|_| DhtError::InvalidLength {
            expected: NODE_ID_LEN,
            actual: value.len(),
        })?;
        Ok(Self(bytes))
    }
}

impl InfoHash {
    #[must_use]
    pub const fn new(value: [u8; INFO_HASH_LEN]) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; INFO_HASH_LEN] {
        &self.0
    }
}

impl TryFrom<&[u8]> for InfoHash {
    type Error = DhtError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let bytes =
            <[u8; INFO_HASH_LEN]>::try_from(value).map_err(|_| DhtError::InvalidLength {
                expected: INFO_HASH_LEN,
                actual: value.len(),
            })?;
        Ok(Self(bytes))
    }
}

impl TransactionId {
    pub fn new(value: Vec<u8>) -> Result<Self, DhtError> {
        if value.len() > MAX_TRANSACTION_ID_LEN {
            return Err(DhtError::TransactionIdTooLong {
                len: value.len(),
                max: MAX_TRANSACTION_ID_LEN,
            });
        }
        Ok(Self(Bytes::from(value)))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl NodeAddr {
    #[must_use]
    pub const fn new(addr: SocketAddr) -> Self {
        Self(addr)
    }

    #[must_use]
    pub const fn socket_addr(&self) -> SocketAddr {
        self.0
    }
}

impl CompactNode {
    pub fn encode_ipv4(&self) -> Result<[u8; IPV4_COMPACT_NODE_LEN], DhtError> {
        let SocketAddr::V4(addr) = self.addr.socket_addr() else {
            return Err(DhtError::NotIpv4);
        };
        let mut output = [0_u8; IPV4_COMPACT_NODE_LEN];
        output[..NODE_ID_LEN].copy_from_slice(self.id.as_bytes());
        output[20..24].copy_from_slice(&addr.ip().octets());
        output[24..26].copy_from_slice(&addr.port().to_be_bytes());
        Ok(output)
    }

    pub fn decode_ipv4(input: &[u8]) -> Result<Self, DhtError> {
        if input.len() != IPV4_COMPACT_NODE_LEN {
            return Err(DhtError::InvalidLength {
                expected: IPV4_COMPACT_NODE_LEN,
                actual: input.len(),
            });
        }
        let id = NodeId::try_from(&input[..NODE_ID_LEN])?;
        let ip = Ipv4Addr::new(input[20], input[21], input[22], input[23]);
        let port = u16::from_be_bytes([input[24], input[25]]);
        Ok(Self {
            id,
            addr: NodeAddr::new(SocketAddr::new(IpAddr::V4(ip), port)),
        })
    }

    pub fn encode_many_ipv4(nodes: &[Self]) -> Result<Bytes, DhtError> {
        let mut output = Vec::with_capacity(nodes.len() * IPV4_COMPACT_NODE_LEN);
        for node in nodes {
            output.extend_from_slice(&node.encode_ipv4()?);
        }
        Ok(Bytes::from(output))
    }

    pub fn decode_many_ipv4(input: &[u8]) -> Result<Vec<Self>, DhtError> {
        if !input.len().is_multiple_of(IPV4_COMPACT_NODE_LEN) {
            return Err(DhtError::InvalidLength {
                expected: IPV4_COMPACT_NODE_LEN,
                actual: input.len(),
            });
        }
        input
            .chunks_exact(IPV4_COMPACT_NODE_LEN)
            .map(Self::decode_ipv4)
            .collect()
    }
}

impl CompactPeer {
    #[must_use]
    pub const fn new(addr: SocketAddr) -> Self {
        Self { addr }
    }

    #[must_use]
    pub const fn socket_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn encode_ipv4(&self) -> Result<[u8; IPV4_COMPACT_PEER_LEN], DhtError> {
        let SocketAddr::V4(addr) = self.addr else {
            return Err(DhtError::NotIpv4);
        };
        let mut output = [0_u8; IPV4_COMPACT_PEER_LEN];
        output[..4].copy_from_slice(&addr.ip().octets());
        output[4..6].copy_from_slice(&addr.port().to_be_bytes());
        Ok(output)
    }

    pub fn decode_ipv4(input: &[u8]) -> Result<Self, DhtError> {
        if input.len() != IPV4_COMPACT_PEER_LEN {
            return Err(DhtError::InvalidLength {
                expected: IPV4_COMPACT_PEER_LEN,
                actual: input.len(),
            });
        }
        let ip = Ipv4Addr::new(input[0], input[1], input[2], input[3]);
        let port = u16::from_be_bytes([input[4], input[5]]);
        Ok(Self::new(SocketAddr::new(IpAddr::V4(ip), port)))
    }
}
