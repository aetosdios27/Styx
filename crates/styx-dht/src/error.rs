use std::io;

use styx_proto::BencodeError;

#[derive(Debug, thiserror::Error)]
pub enum DhtError {
    #[error("invalid length: expected {expected} bytes, got {actual}")]
    InvalidLength { expected: usize, actual: usize },
    #[error("transaction id length {len} exceeds maximum {max}")]
    TransactionIdTooLong { len: usize, max: usize },
    #[error("IPv6 compact encoding is not supported by this function")]
    NotIpv4,
    #[error("IPv4 compact encoding is not supported by this function")]
    NotIpv6,
    #[error("missing KRPC field `{0}`")]
    MissingField(&'static str),
    #[error("invalid KRPC field `{0}`")]
    InvalidField(&'static str),
    #[error("invalid DHT message: {0}")]
    InvalidMessage(&'static str),
    #[error("invalid DHT config field `{0}`")]
    InvalidConfig(&'static str),
    #[error("Kademlia bucket is full")]
    BucketFull,
    #[error("node is unknown")]
    UnknownNode,
    #[error("peer store is full")]
    PeerStoreFull,
    #[error("transaction table is full")]
    TransactionTableFull,
    #[error("unexpected or unsolicited transaction")]
    UnexpectedTransaction,
    #[error("invalid announce token")]
    InvalidToken,
    #[error(transparent)]
    Bencode(#[from] BencodeError),
    #[error("integer conversion overflow")]
    IntegerOverflow,
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl PartialEq for DhtError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::InvalidLength {
                    expected: left_expected,
                    actual: left_actual,
                },
                Self::InvalidLength {
                    expected: right_expected,
                    actual: right_actual,
                },
            ) => left_expected == right_expected && left_actual == right_actual,
            (
                Self::TransactionIdTooLong {
                    len: left_len,
                    max: left_max,
                },
                Self::TransactionIdTooLong {
                    len: right_len,
                    max: right_max,
                },
            ) => left_len == right_len && left_max == right_max,
            (Self::NotIpv4, Self::NotIpv4)
            | (Self::NotIpv6, Self::NotIpv6)
            | (Self::IntegerOverflow, Self::IntegerOverflow)
            | (Self::BucketFull, Self::BucketFull)
            | (Self::UnknownNode, Self::UnknownNode)
            | (Self::PeerStoreFull, Self::PeerStoreFull)
            | (Self::TransactionTableFull, Self::TransactionTableFull)
            | (Self::UnexpectedTransaction, Self::UnexpectedTransaction)
            | (Self::InvalidToken, Self::InvalidToken) => true,
            (Self::MissingField(left), Self::MissingField(right))
            | (Self::InvalidField(left), Self::InvalidField(right))
            | (Self::InvalidMessage(left), Self::InvalidMessage(right))
            | (Self::InvalidConfig(left), Self::InvalidConfig(right)) => left == right,
            (Self::Bencode(left), Self::Bencode(right)) => left == right,
            (Self::Io(left), Self::Io(right)) => left.kind() == right.kind(),
            _ => false,
        }
    }
}

impl Eq for DhtError {}
