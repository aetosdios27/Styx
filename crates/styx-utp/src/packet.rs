use bytes::{BufMut, Bytes, BytesMut};

use crate::{
    ConnectionId, PacketType, SelectiveAck, SeqNr, TimestampMicros, UtpError, WindowBytes,
    HEADER_LEN, MAX_EXTENSION_BYTES, MAX_PACKET_SIZE, UTP_VERSION,
};

const EXTENSION_SELECTIVE_ACK: u8 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Extension {
    SelectiveAck(Bytes),
    Unknown { kind: u8, bytes: Bytes },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UtpPacket {
    packet_type: PacketType,
    connection_id: ConnectionId,
    timestamp: TimestampMicros,
    timestamp_diff: TimestampMicros,
    wnd_size: WindowBytes,
    seq_nr: SeqNr,
    ack_nr: SeqNr,
    extensions: Vec<Extension>,
    payload: Bytes,
}

impl UtpPacket {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        packet_type: PacketType,
        connection_id: ConnectionId,
        timestamp: TimestampMicros,
        timestamp_diff: TimestampMicros,
        wnd_size: WindowBytes,
        seq_nr: SeqNr,
        ack_nr: SeqNr,
        extensions: Vec<Extension>,
        payload: Bytes,
    ) -> Self {
        Self {
            packet_type,
            connection_id,
            timestamp,
            timestamp_diff,
            wnd_size,
            seq_nr,
            ack_nr,
            extensions,
            payload,
        }
    }

    pub fn decode(input: &[u8]) -> Result<Self, UtpError> {
        if input.len() < HEADER_LEN {
            return Err(UtpError::PacketTooShort { len: input.len() });
        }
        if input.len() > MAX_PACKET_SIZE {
            return Err(UtpError::PacketTooLarge {
                len: input.len(),
                max: MAX_PACKET_SIZE,
            });
        }

        let packet_type = PacketType::try_from(input[0] >> 4)?;
        let version = input[0] & 0x0f;
        if version != UTP_VERSION {
            return Err(UtpError::UnsupportedVersion { version });
        }

        let first_extension = input[1];
        let connection_id = ConnectionId::new(u16::from_be_bytes([input[2], input[3]]));
        let timestamp =
            TimestampMicros::new(u32::from_be_bytes([input[4], input[5], input[6], input[7]]));
        let timestamp_diff = TimestampMicros::new(u32::from_be_bytes([
            input[8], input[9], input[10], input[11],
        ]));
        let wnd_size = WindowBytes::new(u32::from_be_bytes([
            input[12], input[13], input[14], input[15],
        ]));
        let seq_nr = SeqNr::new(u16::from_be_bytes([input[16], input[17]]));
        let ack_nr = SeqNr::new(u16::from_be_bytes([input[18], input[19]]));

        let mut cursor = HEADER_LEN;
        let mut extension_kind = first_extension;
        let mut extensions = Vec::new();
        let mut extension_bytes = 0usize;
        while extension_kind != 0 {
            if input.len().saturating_sub(cursor) < 2 {
                return Err(UtpError::InvalidExtensionLength { len: 2 });
            }
            let next_extension = input[cursor];
            let len = input[cursor + 1] as usize;
            cursor += 2;
            extension_bytes += 2 + len;
            if extension_bytes > MAX_EXTENSION_BYTES {
                return Err(UtpError::ExtensionChainTooLarge {
                    len: extension_bytes,
                    max: MAX_EXTENSION_BYTES,
                });
            }
            if input.len().saturating_sub(cursor) < len {
                return Err(UtpError::InvalidExtensionLength { len });
            }
            let bytes = Bytes::copy_from_slice(&input[cursor..cursor + len]);
            if extension_kind == EXTENSION_SELECTIVE_ACK {
                let _ = SelectiveAck::parse(ack_nr, &bytes)?;
                extensions.push(Extension::SelectiveAck(bytes));
            } else {
                extensions.push(Extension::Unknown {
                    kind: extension_kind,
                    bytes,
                });
            }
            cursor += len;
            extension_kind = next_extension;
        }

        Ok(Self {
            packet_type,
            connection_id,
            timestamp,
            timestamp_diff,
            wnd_size,
            seq_nr,
            ack_nr,
            extensions,
            payload: Bytes::copy_from_slice(&input[cursor..]),
        })
    }

    #[must_use]
    pub fn encode(&self) -> Bytes {
        let mut bytes =
            BytesMut::with_capacity(HEADER_LEN + self.extension_len() + self.payload.len());
        bytes.put_u8((self.packet_type.as_u8() << 4) | UTP_VERSION);
        bytes.put_u8(self.extensions.first().map_or(0, Extension::kind));
        bytes.put_u16(self.connection_id.get());
        bytes.put_u32(self.timestamp.get());
        bytes.put_u32(self.timestamp_diff.get());
        bytes.put_u32(self.wnd_size.get());
        bytes.put_u16(self.seq_nr.get());
        bytes.put_u16(self.ack_nr.get());

        for (index, extension) in self.extensions.iter().enumerate() {
            let next = self.extensions.get(index + 1).map_or(0, Extension::kind);
            bytes.put_u8(next);
            bytes.put_u8(extension.bytes().len() as u8);
            bytes.extend_from_slice(extension.bytes());
        }

        bytes.extend_from_slice(&self.payload);
        bytes.freeze()
    }

    #[must_use]
    pub const fn packet_type(&self) -> PacketType {
        self.packet_type
    }

    #[must_use]
    pub const fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }

    #[must_use]
    pub const fn timestamp(&self) -> TimestampMicros {
        self.timestamp
    }

    #[must_use]
    pub const fn timestamp_diff(&self) -> TimestampMicros {
        self.timestamp_diff
    }

    #[must_use]
    pub const fn wnd_size(&self) -> WindowBytes {
        self.wnd_size
    }

    #[must_use]
    pub const fn seq_nr(&self) -> SeqNr {
        self.seq_nr
    }

    #[must_use]
    pub const fn ack_nr(&self) -> SeqNr {
        self.ack_nr
    }

    pub fn set_ack_nr(&mut self, ack_nr: SeqNr) {
        self.ack_nr = ack_nr;
    }

    #[must_use]
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    #[must_use]
    pub fn extensions(&self) -> &[Extension] {
        &self.extensions
    }

    pub fn selective_ack(&self) -> Option<SelectiveAck> {
        self.extensions.iter().find_map(|extension| {
            if let Extension::SelectiveAck(bytes) = extension {
                SelectiveAck::parse(self.ack_nr, bytes).ok()
            } else {
                None
            }
        })
    }

    fn extension_len(&self) -> usize {
        self.extensions
            .iter()
            .map(|extension| 2 + extension.bytes().len())
            .sum()
    }
}

impl Extension {
    #[must_use]
    pub const fn kind(&self) -> u8 {
        match self {
            Self::SelectiveAck(_) => EXTENSION_SELECTIVE_ACK,
            Self::Unknown { kind, .. } => *kind,
        }
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::SelectiveAck(bytes) | Self::Unknown { bytes, .. } => bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use crate::{
        ConnectionId, PacketType, SeqNr, TimestampMicros, UtpError, UtpPacket, WindowBytes,
        MAX_EXTENSION_BYTES, MAX_PACKET_SIZE,
    };

    #[test]
    fn packet_round_trips_header_extensions_and_payload() {
        let packet = UtpPacket::new(
            PacketType::Data,
            ConnectionId::new(7),
            TimestampMicros::new(100),
            TimestampMicros::new(25),
            WindowBytes::new(65_535),
            SeqNr::new(10),
            SeqNr::new(9),
            vec![crate::Extension::Unknown {
                kind: 99,
                bytes: Bytes::from_static(&[1, 2, 3, 4]),
            }],
            Bytes::from_static(b"hello"),
        );

        let decoded = UtpPacket::decode(&packet.encode()).unwrap();

        assert_eq!(decoded, packet);
    }

    #[test]
    fn decode_rejects_short_packet() {
        let err = UtpPacket::decode(&[0; 19]).unwrap_err();

        assert_eq!(err, UtpError::PacketTooShort { len: 19 });
    }

    #[test]
    fn decode_rejects_oversized_packet() {
        let err = UtpPacket::decode(&vec![0; MAX_PACKET_SIZE + 1]).unwrap_err();

        assert_eq!(
            err,
            UtpError::PacketTooLarge {
                len: MAX_PACKET_SIZE + 1,
                max: MAX_PACKET_SIZE
            }
        );
    }

    #[test]
    fn decode_rejects_unsupported_version() {
        let mut bytes = vec![0; 20];
        bytes[0] = 2;

        let err = UtpPacket::decode(&bytes).unwrap_err();

        assert_eq!(err, UtpError::UnsupportedVersion { version: 2 });
    }

    #[test]
    fn decode_rejects_unknown_packet_type() {
        let mut bytes = vec![0; 20];
        bytes[0] = (9 << 4) | 1;

        let err = UtpPacket::decode(&bytes).unwrap_err();

        assert_eq!(err, UtpError::UnknownPacketType { value: 9 });
    }

    #[test]
    fn decode_rejects_extension_chain_that_exceeds_packet() {
        let mut bytes = vec![0; 20];
        bytes[0] = 1;
        bytes[1] = 1;
        bytes.extend_from_slice(&[0, 4, 1]);

        let err = UtpPacket::decode(&bytes).unwrap_err();

        assert_eq!(err, UtpError::InvalidExtensionLength { len: 4 });
    }

    #[test]
    fn decode_rejects_extension_chain_over_cap() {
        let mut bytes = vec![0; 20];
        bytes[0] = 1;
        bytes[1] = 1;
        bytes.extend_from_slice(&[0, 255]);
        bytes.extend(std::iter::repeat_n(0, 255));

        let err = UtpPacket::decode(&bytes).unwrap_err();

        assert_eq!(
            err,
            UtpError::ExtensionChainTooLarge {
                len: 257,
                max: MAX_EXTENSION_BYTES
            }
        );
    }

    #[test]
    fn arbitrary_short_packets_do_not_panic() {
        for len in 0..=32 {
            let input = vec![0xa5; len];
            let _ = UtpPacket::decode(&input);
        }
    }
}
