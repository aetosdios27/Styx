#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use bytes::Bytes;

    use crate::{
        ConnectionId, PacketType, ReorderBuffer, RetransmitQueue, SeqNr, TimestampMicros, UtpError,
        UtpPacket, WindowBytes, MAX_EXTENSION_BYTES, MAX_PACKET_SIZE,
    };

    fn packet(payload: Bytes) -> UtpPacket {
        UtpPacket::new(
            PacketType::Data,
            ConnectionId::new(1),
            TimestampMicros::new(0),
            TimestampMicros::new(0),
            WindowBytes::new(0),
            SeqNr::new(1),
            SeqNr::new(0),
            Vec::new(),
            payload,
        )
    }

    #[test]
    fn oversized_packet_returns_packet_too_large() {
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
    fn extension_chain_cap_returns_typed_error() {
        let mut bytes = vec![0; 20];
        bytes[0] = 1;
        bytes[1] = 2;
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
    fn reorder_buffer_byte_cap_returns_resource_error() {
        let mut buffer = ReorderBuffer::new(SeqNr::new(1), 1);

        let err = buffer
            .push(SeqNr::new(3), Bytes::from_static(b"xx"))
            .unwrap_err();

        assert_eq!(
            err,
            UtpError::ResourceLimitExceeded {
                resource: "reorder_buffer"
            }
        );
    }

    #[test]
    fn retransmit_queue_byte_cap_returns_resource_error() {
        let now = Instant::now();
        let mut queue = RetransmitQueue::with_byte_cap(1);

        let err = queue
            .try_push(
                packet(Bytes::from_static(b"xx")),
                now,
                now + Duration::from_secs(1),
            )
            .unwrap_err();

        assert_eq!(
            err,
            UtpError::ResourceLimitExceeded {
                resource: "retransmit_queue"
            }
        );
    }
}
