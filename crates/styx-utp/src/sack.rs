use crate::{SeqNr, UtpError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectiveAck {
    acked: Vec<SeqNr>,
}

impl SelectiveAck {
    pub fn parse(ack_nr: SeqNr, bytes: &[u8]) -> Result<Self, UtpError> {
        Self::parse_with_window(ack_nr, bytes, usize::MAX)
    }

    pub fn parse_with_window(
        ack_nr: SeqNr,
        bytes: &[u8],
        max_sequences: usize,
    ) -> Result<Self, UtpError> {
        if bytes.len() < 4 || !bytes.len().is_multiple_of(4) {
            return Err(UtpError::InvalidExtensionLength { len: bytes.len() });
        }

        let mut acked = Vec::new();
        for (byte_index, byte) in bytes.iter().enumerate() {
            for bit in 0..8 {
                let distance = 2 + byte_index * 8 + bit;
                if max_sequences != usize::MAX && distance > max_sequences + 1 {
                    continue;
                }
                if byte & (1 << bit) != 0 {
                    acked.push(ack_nr.wrapping_add(distance as u16));
                }
            }
        }
        Ok(Self { acked })
    }

    #[must_use]
    pub fn acked(&self) -> &[SeqNr] {
        &self.acked
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use crate::{Extension, SelectiveAck, SeqNr, UtpError, UtpPacket};

    #[test]
    fn bit_zero_acknowledges_ack_nr_plus_two() {
        let sack = SelectiveAck::parse(SeqNr::new(10), &[0b0000_0001, 0, 0, 0]).unwrap();

        assert!(sack.acked().contains(&SeqNr::new(12)));
    }

    #[test]
    fn multiple_bytes_map_to_increasing_sequence_numbers() {
        let sack = SelectiveAck::parse(SeqNr::new(10), &[0b0000_0001, 0b0000_0001, 0, 0]).unwrap();

        assert!(sack.acked().contains(&SeqNr::new(12)));
        assert!(sack.acked().contains(&SeqNr::new(20)));
    }

    #[test]
    fn parse_rejects_sack_length_that_is_not_multiple_of_four() {
        let err = SelectiveAck::parse(SeqNr::new(10), &[1, 2, 3]).unwrap_err();

        assert_eq!(err, UtpError::InvalidExtensionLength { len: 3 });
    }

    #[test]
    fn parse_ignores_bits_outside_tracked_window() {
        let sack =
            SelectiveAck::parse_with_window(SeqNr::new(10), &[0xff, 0xff, 0xff, 0xff], 4).unwrap();

        assert!(sack
            .acked()
            .iter()
            .all(|seq| seq.forward_distance_from(SeqNr::new(10)) <= 5));
    }

    #[test]
    fn sack_extension_round_trips_through_packet() {
        let packet = UtpPacket::new(
            crate::PacketType::State,
            crate::ConnectionId::new(1),
            crate::TimestampMicros::new(1),
            crate::TimestampMicros::new(0),
            crate::WindowBytes::new(1024),
            SeqNr::new(20),
            SeqNr::new(10),
            vec![
                Extension::Unknown {
                    kind: 99,
                    bytes: Bytes::from_static(&[7, 7, 7, 7]),
                },
                Extension::SelectiveAck(Bytes::from_static(&[1, 0, 0, 0])),
            ],
            Bytes::new(),
        );

        let decoded = UtpPacket::decode(&packet.encode()).unwrap();

        assert_eq!(decoded.selective_ack().unwrap().acked(), &[SeqNr::new(12)]);
    }
}
