use crate::{ConnectionId, SeqNr};

#[derive(Debug, thiserror::Error)]
pub enum UtpError {
    #[error("uTP packet too short: {len} bytes")]
    PacketTooShort { len: usize },
    #[error("uTP packet too large: {len} bytes exceeds max {max}")]
    PacketTooLarge { len: usize, max: usize },
    #[error("unsupported uTP version {version}")]
    UnsupportedVersion { version: u8 },
    #[error("unknown uTP packet type {value}")]
    UnknownPacketType { value: u8 },
    #[error("invalid uTP extension length {len}")]
    InvalidExtensionLength { len: usize },
    #[error("uTP extension chain too large: {len} bytes exceeds max {max}")]
    ExtensionChainTooLarge { len: usize, max: usize },
    #[error("invalid uTP state transition")]
    InvalidStateTransition,
    #[error("uTP connection id mismatch: expected {expected:?}, got {actual:?}")]
    ConnectionIdMismatch {
        expected: ConnectionId,
        actual: ConnectionId,
    },
    #[error("uTP sequence out of window: {seq:?}")]
    SequenceOutOfWindow { seq: SeqNr },
    #[error("uTP send window is full")]
    SendWindowFull,
    #[error("uTP resource limit exceeded: {resource}")]
    ResourceLimitExceeded { resource: &'static str },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl PartialEq for UtpError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::PacketTooShort { len: left }, Self::PacketTooShort { len: right }) => {
                left == right
            }
            (
                Self::PacketTooLarge {
                    len: left_len,
                    max: left_max,
                },
                Self::PacketTooLarge {
                    len: right_len,
                    max: right_max,
                },
            ) => left_len == right_len && left_max == right_max,
            (
                Self::UnsupportedVersion { version: left },
                Self::UnsupportedVersion { version: right },
            )
            | (Self::UnknownPacketType { value: left }, Self::UnknownPacketType { value: right }) => {
                left == right
            }
            (
                Self::InvalidExtensionLength { len: left },
                Self::InvalidExtensionLength { len: right },
            ) => left == right,
            (
                Self::ExtensionChainTooLarge {
                    len: left_len,
                    max: left_max,
                },
                Self::ExtensionChainTooLarge {
                    len: right_len,
                    max: right_max,
                },
            ) => left_len == right_len && left_max == right_max,
            (Self::InvalidStateTransition, Self::InvalidStateTransition)
            | (Self::SendWindowFull, Self::SendWindowFull) => true,
            (
                Self::ConnectionIdMismatch {
                    expected: left_expected,
                    actual: left_actual,
                },
                Self::ConnectionIdMismatch {
                    expected: right_expected,
                    actual: right_actual,
                },
            ) => left_expected == right_expected && left_actual == right_actual,
            (Self::SequenceOutOfWindow { seq: left }, Self::SequenceOutOfWindow { seq: right }) => {
                left == right
            }
            (
                Self::ResourceLimitExceeded { resource: left },
                Self::ResourceLimitExceeded { resource: right },
            ) => left == right,
            (Self::Io(left), Self::Io(right)) => left.kind() == right.kind(),
            _ => false,
        }
    }
}

impl Eq for UtpError {}

#[cfg(test)]
mod tests {}
