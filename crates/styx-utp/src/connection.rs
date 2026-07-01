use std::time::Instant;

use bytes::Bytes;

use crate::{
    ConnectionId, PacketType, ReorderBuffer, SeqNr, TimestampMicros, UtpError, UtpPacket,
    WindowBytes,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionRole {
    Client,
    Server,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    SynSent,
    Connected,
    FinSent,
    Reset,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UtpEvent {
    Connected,
    Data(Bytes),
    Eof,
    Packet(UtpPacket),
    Reset,
}

#[derive(Clone, Debug)]
pub struct UtpConnection {
    role: ConnectionRole,
    state: ConnectionState,
    recv_id: ConnectionId,
    send_id: ConnectionId,
    next_seq: SeqNr,
    ack_nr: SeqNr,
    recv: ReorderBuffer,
    outgoing: Vec<UtpPacket>,
    send_window_bytes: usize,
    bytes_in_flight: usize,
}

impl UtpConnection {
    pub fn client_syn(
        _now: Instant,
        recv_id: ConnectionId,
        initial_seq: SeqNr,
    ) -> Result<(Self, UtpPacket), UtpError> {
        let syn = UtpPacket::new(
            PacketType::Syn,
            recv_id,
            TimestampMicros::new(0),
            TimestampMicros::new(0),
            WindowBytes::new(0),
            initial_seq,
            SeqNr::new(0),
            Vec::new(),
            Bytes::new(),
        );
        Ok((
            Self {
                role: ConnectionRole::Client,
                state: ConnectionState::SynSent,
                recv_id,
                send_id: recv_id.wrapping_add(1),
                next_seq: initial_seq.wrapping_add(1),
                ack_nr: SeqNr::new(0),
                recv: ReorderBuffer::new(SeqNr::new(0), 1024 * 1024),
                outgoing: Vec::new(),
                send_window_bytes: usize::MAX,
                bytes_in_flight: 0,
            },
            syn,
        ))
    }

    pub fn server_accept(
        syn: UtpPacket,
        _now: Instant,
        initial_seq: SeqNr,
    ) -> Result<(Self, UtpPacket), UtpError> {
        if syn.packet_type() != PacketType::Syn {
            return Err(UtpError::InvalidStateTransition);
        }
        let recv_id = syn.connection_id().wrapping_add(1);
        let send_id = syn.connection_id();
        let state = UtpPacket::new(
            PacketType::State,
            send_id,
            TimestampMicros::new(0),
            TimestampMicros::new(0),
            WindowBytes::new(0),
            initial_seq,
            syn.seq_nr(),
            Vec::new(),
            Bytes::new(),
        );
        Ok((
            Self {
                role: ConnectionRole::Server,
                state: ConnectionState::Connected,
                recv_id,
                send_id,
                next_seq: initial_seq.wrapping_add(1),
                ack_nr: syn.seq_nr(),
                recv: ReorderBuffer::new(syn.seq_nr().wrapping_add(1), 1024 * 1024),
                outgoing: Vec::new(),
                send_window_bytes: usize::MAX,
                bytes_in_flight: 0,
            },
            state,
        ))
    }

    pub fn handle_packet(
        &mut self,
        packet: UtpPacket,
        _now: Instant,
    ) -> Result<Vec<UtpEvent>, UtpError> {
        self.validate_connection_id(&packet)?;
        match packet.packet_type() {
            PacketType::State if self.state == ConnectionState::SynSent => {
                self.state = ConnectionState::Connected;
                self.ack_nr = packet.seq_nr();
                Ok(vec![UtpEvent::Connected])
            }
            PacketType::Data => {
                let outcome = self.recv.push(packet.seq_nr(), packet.payload().clone())?;
                self.ack_nr = packet.seq_nr();
                let mut events = outcome
                    .data
                    .into_iter()
                    .map(UtpEvent::Data)
                    .collect::<Vec<_>>();
                events.push(UtpEvent::Packet(self.state_packet()));
                Ok(events)
            }
            PacketType::Fin => {
                self.state = ConnectionState::Closed;
                Ok(vec![UtpEvent::Eof])
            }
            PacketType::Reset => {
                self.state = ConnectionState::Reset;
                self.outgoing.clear();
                Ok(vec![UtpEvent::Reset])
            }
            _ => Err(UtpError::InvalidStateTransition),
        }
    }

    pub fn queue_send(&mut self, payload: Bytes, _now: Instant) -> Result<(), UtpError> {
        if self.state != ConnectionState::Connected {
            return Err(UtpError::InvalidStateTransition);
        }
        if self.bytes_in_flight + payload.len() > self.send_window_bytes {
            return Err(UtpError::SendWindowFull);
        }
        let packet = UtpPacket::new(
            PacketType::Data,
            self.send_id,
            TimestampMicros::new(0),
            TimestampMicros::new(0),
            WindowBytes::new(0),
            self.next_seq,
            self.ack_nr,
            Vec::new(),
            payload,
        );
        self.next_seq = self.next_seq.wrapping_add(1);
        self.bytes_in_flight += packet.payload().len();
        self.outgoing.push(packet);
        Ok(())
    }

    #[must_use]
    pub fn poll_transmit(&mut self, _now: Instant) -> Vec<UtpPacket> {
        std::mem::take(&mut self.outgoing)
    }

    pub fn close(&mut self) -> Result<UtpPacket, UtpError> {
        if self.state != ConnectionState::Connected {
            return Err(UtpError::InvalidStateTransition);
        }
        self.state = ConnectionState::FinSent;
        Ok(UtpPacket::new(
            PacketType::Fin,
            self.send_id,
            TimestampMicros::new(0),
            TimestampMicros::new(0),
            WindowBytes::new(0),
            self.next_seq,
            self.ack_nr,
            Vec::new(),
            Bytes::new(),
        ))
    }

    #[must_use]
    pub const fn state(&self) -> ConnectionState {
        self.state
    }

    #[must_use]
    pub const fn role(&self) -> ConnectionRole {
        self.role
    }

    #[must_use]
    pub const fn recv_id(&self) -> ConnectionId {
        self.recv_id
    }

    #[must_use]
    pub const fn send_id(&self) -> ConnectionId {
        self.send_id
    }

    pub fn set_send_window_bytes(&mut self, bytes: usize) {
        self.send_window_bytes = bytes;
    }

    fn state_packet(&self) -> UtpPacket {
        UtpPacket::new(
            PacketType::State,
            self.send_id,
            TimestampMicros::new(0),
            TimestampMicros::new(0),
            WindowBytes::new(0),
            self.next_seq,
            self.ack_nr,
            Vec::new(),
            Bytes::new(),
        )
    }

    fn validate_connection_id(&self, packet: &UtpPacket) -> Result<(), UtpError> {
        let expected = match packet.packet_type() {
            PacketType::State if self.state == ConnectionState::SynSent => self.recv_id,
            PacketType::Data | PacketType::Fin | PacketType::Reset | PacketType::State => {
                self.recv_id
            }
            PacketType::Syn => self.recv_id,
        };
        if packet.connection_id() != expected {
            return Err(UtpError::ConnectionIdMismatch {
                expected,
                actual: packet.connection_id(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use bytes::Bytes;

    use super::*;

    #[test]
    fn client_syn_emits_syn_with_receive_connection_id() {
        let (_conn, syn) =
            UtpConnection::client_syn(Instant::now(), ConnectionId::new(10), SeqNr::new(1))
                .unwrap();

        assert_eq!(syn.packet_type(), PacketType::Syn);
        assert_eq!(syn.connection_id(), ConnectionId::new(10));
    }

    #[test]
    fn server_accept_emits_state_and_sets_data_ids() {
        let (_client, syn) =
            UtpConnection::client_syn(Instant::now(), ConnectionId::new(10), SeqNr::new(1))
                .unwrap();

        let (server, state) =
            UtpConnection::server_accept(syn, Instant::now(), SeqNr::new(100)).unwrap();

        assert_eq!(state.packet_type(), PacketType::State);
        assert_eq!(state.connection_id(), ConnectionId::new(10));
        assert_eq!(server.send_id(), ConnectionId::new(10));
        assert_eq!(server.recv_id(), ConnectionId::new(11));
    }

    #[test]
    fn client_enters_connected_on_matching_state() {
        let (mut client, syn) =
            UtpConnection::client_syn(Instant::now(), ConnectionId::new(10), SeqNr::new(1))
                .unwrap();
        let (_server, state) =
            UtpConnection::server_accept(syn, Instant::now(), SeqNr::new(100)).unwrap();

        let events = client.handle_packet(state, Instant::now()).unwrap();

        assert_eq!(events, vec![UtpEvent::Connected]);
        assert_eq!(client.state(), ConnectionState::Connected);
    }

    #[test]
    fn client_data_uses_receive_id_plus_one_after_handshake() {
        let (mut client, syn) =
            UtpConnection::client_syn(Instant::now(), ConnectionId::new(10), SeqNr::new(1))
                .unwrap();
        let (_server, state) =
            UtpConnection::server_accept(syn, Instant::now(), SeqNr::new(100)).unwrap();
        client.handle_packet(state, Instant::now()).unwrap();

        client
            .queue_send(Bytes::from_static(b"abc"), Instant::now())
            .unwrap();
        let packet = client.poll_transmit(Instant::now()).remove(0);

        assert_eq!(packet.connection_id(), ConnectionId::new(11));
    }

    #[test]
    fn wrong_connection_id_returns_mismatch() {
        let (mut client, _syn) =
            UtpConnection::client_syn(Instant::now(), ConnectionId::new(10), SeqNr::new(1))
                .unwrap();
        let packet = UtpPacket::new(
            PacketType::State,
            ConnectionId::new(99),
            TimestampMicros::new(0),
            TimestampMicros::new(0),
            WindowBytes::new(0),
            SeqNr::new(1),
            SeqNr::new(1),
            Vec::new(),
            Bytes::new(),
        );

        let err = client.handle_packet(packet, Instant::now()).unwrap_err();

        assert_eq!(
            err,
            UtpError::ConnectionIdMismatch {
                expected: ConnectionId::new(10),
                actual: ConnectionId::new(99)
            }
        );
    }

    #[test]
    fn local_close_emits_fin_and_enters_fin_sent() {
        let (mut client, syn) =
            UtpConnection::client_syn(Instant::now(), ConnectionId::new(10), SeqNr::new(1))
                .unwrap();
        let (_server, state) =
            UtpConnection::server_accept(syn, Instant::now(), SeqNr::new(100)).unwrap();
        client.handle_packet(state, Instant::now()).unwrap();

        let fin = client.close().unwrap();

        assert_eq!(fin.packet_type(), PacketType::Fin);
        assert_eq!(client.state(), ConnectionState::FinSent);
    }

    #[test]
    fn receiving_reset_closes_immediately() {
        let (mut client, syn) =
            UtpConnection::client_syn(Instant::now(), ConnectionId::new(10), SeqNr::new(1))
                .unwrap();
        let (_server, state) =
            UtpConnection::server_accept(syn, Instant::now(), SeqNr::new(100)).unwrap();
        client.handle_packet(state, Instant::now()).unwrap();
        let reset = UtpPacket::new(
            PacketType::Reset,
            ConnectionId::new(10),
            TimestampMicros::new(0),
            TimestampMicros::new(0),
            WindowBytes::new(0),
            SeqNr::new(2),
            SeqNr::new(1),
            Vec::new(),
            Bytes::new(),
        );

        let events = client.handle_packet(reset, Instant::now()).unwrap();

        assert_eq!(events, vec![UtpEvent::Reset]);
        assert_eq!(client.state(), ConnectionState::Reset);
    }
}
