use std::{convert::TryFrom, time::Duration};

use crate::UtpError;

pub const UTP_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 20;
pub const TARGET_DELAY: Duration = Duration::from_millis(100);
pub const INITIAL_TIMEOUT: Duration = Duration::from_secs(1);
pub const MIN_TIMEOUT: Duration = Duration::from_millis(500);
pub const DEFAULT_MTU: usize = 1200;
pub const MAX_PACKET_SIZE: usize = 1500;
pub const MAX_EXTENSION_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PacketType {
    Data,
    Fin,
    State,
    Reset,
    Syn,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectionId(u16);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SeqNr(u16);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TimestampMicros(u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WindowBytes(u32);

impl PacketType {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Data => 0,
            Self::Fin => 1,
            Self::State => 2,
            Self::Reset => 3,
            Self::Syn => 4,
        }
    }
}

impl TryFrom<u8> for PacketType {
    type Error = UtpError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Data),
            1 => Ok(Self::Fin),
            2 => Ok(Self::State),
            3 => Ok(Self::Reset),
            4 => Ok(Self::Syn),
            value => Err(UtpError::UnknownPacketType { value }),
        }
    }
}

impl ConnectionId {
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    #[must_use]
    pub const fn wrapping_add(self, value: u16) -> Self {
        Self(self.0.wrapping_add(value))
    }
}

impl SeqNr {
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    #[must_use]
    pub const fn wrapping_add(self, value: u16) -> Self {
        Self(self.0.wrapping_add(value))
    }

    #[must_use]
    pub const fn forward_distance_to(self, other: Self) -> u16 {
        other.0.wrapping_sub(self.0)
    }

    #[must_use]
    pub const fn forward_distance_from(self, base: Self) -> u16 {
        base.forward_distance_to(self)
    }
}

impl TimestampMicros {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl WindowBytes {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use crate::{PacketType, SeqNr, UtpError};

    #[test]
    fn packet_type_try_from_accepts_bep29_values() {
        assert_eq!(PacketType::try_from(0).unwrap(), PacketType::Data);
        assert_eq!(PacketType::try_from(1).unwrap(), PacketType::Fin);
        assert_eq!(PacketType::try_from(2).unwrap(), PacketType::State);
        assert_eq!(PacketType::try_from(3).unwrap(), PacketType::Reset);
        assert_eq!(PacketType::try_from(4).unwrap(), PacketType::Syn);
    }

    #[test]
    fn packet_type_try_from_rejects_unknown_values() {
        let err = PacketType::try_from(9).unwrap_err();

        assert_eq!(err, UtpError::UnknownPacketType { value: 9 });
    }

    #[test]
    fn seq_nr_wraps_and_computes_forward_distance() {
        let seq = SeqNr::new(u16::MAX);

        assert_eq!(seq.wrapping_add(1), SeqNr::new(0));
        assert_eq!(SeqNr::new(u16::MAX).forward_distance_to(SeqNr::new(1)), 2);
    }
}
