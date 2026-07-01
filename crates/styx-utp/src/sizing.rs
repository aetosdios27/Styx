use crate::{DEFAULT_MTU, HEADER_LEN, MAX_PACKET_SIZE};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketSizer {
    mtu: usize,
    max_packet_size: usize,
}

impl Default for PacketSizer {
    fn default() -> Self {
        Self {
            mtu: DEFAULT_MTU,
            max_packet_size: MAX_PACKET_SIZE,
        }
    }
}

impl PacketSizer {
    #[must_use]
    pub const fn new(mtu: usize, max_packet_size: usize) -> Self {
        Self {
            mtu,
            max_packet_size,
        }
    }

    #[must_use]
    pub fn max_payload(
        self,
        extension_overhead: usize,
        congestion_window: usize,
        remote_window: usize,
        bytes_in_flight: usize,
    ) -> usize {
        let packet_limit = self.mtu.min(self.max_packet_size);
        let header_budget = HEADER_LEN + extension_overhead;
        let mtu_payload = packet_limit.saturating_sub(header_budget);
        let allowed_window = congestion_window.min(remote_window);
        let window_payload = allowed_window.saturating_sub(bytes_in_flight);
        mtu_payload.min(window_payload)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use bytes::Bytes;

    use crate::{
        ConnectionId, SeqNr, UtpConnection, UtpError, DEFAULT_MTU, HEADER_LEN, MAX_PACKET_SIZE,
    };

    use super::*;

    #[test]
    fn default_max_payload_is_below_mtu_minus_header() {
        let payload = PacketSizer::default().max_payload(0, usize::MAX, usize::MAX, 0);

        assert_eq!(payload, DEFAULT_MTU - HEADER_LEN);
    }

    #[test]
    fn max_payload_shrinks_when_congestion_window_is_small() {
        let payload = PacketSizer::default().max_payload(0, 100, usize::MAX, 40);

        assert_eq!(payload, 60);
    }

    #[test]
    fn queue_send_rejects_payload_past_send_window() {
        let (mut conn, syn) =
            UtpConnection::client_syn(Instant::now(), ConnectionId::new(1), SeqNr::new(1)).unwrap();
        let (_server, state) =
            UtpConnection::server_accept(syn, Instant::now(), SeqNr::new(10)).unwrap();
        conn.handle_packet(state, Instant::now()).unwrap();
        conn.set_send_window_bytes(2);

        let err = conn
            .queue_send(Bytes::from_static(b"abc"), Instant::now())
            .unwrap_err();

        assert_eq!(err, UtpError::SendWindowFull);
    }

    #[test]
    fn max_payload_never_exceeds_max_packet_size_budget() {
        let payload = PacketSizer::new(MAX_PACKET_SIZE + 1000, MAX_PACKET_SIZE).max_payload(
            0,
            usize::MAX,
            usize::MAX,
            0,
        );

        assert_eq!(payload, MAX_PACKET_SIZE - HEADER_LEN);
    }
}
