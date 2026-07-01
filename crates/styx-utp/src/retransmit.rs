use std::{collections::BTreeMap, time::Instant};

use crate::{SelectiveAck, SeqNr, UtpError, UtpPacket};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SentPacket {
    pub packet: UtpPacket,
    pub sent_at: Instant,
    pub deadline: Instant,
    pub retransmissions: u32,
}

#[derive(Clone, Debug)]
pub struct RetransmitQueue {
    packets: BTreeMap<SeqNr, SentPacket>,
    bytes_in_flight: usize,
    byte_cap: usize,
}

impl RetransmitQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, packet: UtpPacket, sent_at: Instant, deadline: Instant) {
        let _ = self.try_push(packet, sent_at, deadline);
    }

    pub fn try_push(
        &mut self,
        packet: UtpPacket,
        sent_at: Instant,
        deadline: Instant,
    ) -> Result<(), UtpError> {
        if self.bytes_in_flight + packet.payload().len() > self.byte_cap {
            return Err(UtpError::ResourceLimitExceeded {
                resource: "retransmit_queue",
            });
        }
        self.bytes_in_flight += packet.payload().len();
        self.packets.insert(
            packet.seq_nr(),
            SentPacket {
                packet,
                sent_at,
                deadline,
                retransmissions: 0,
            },
        );
        Ok(())
    }

    #[must_use]
    pub fn with_byte_cap(byte_cap: usize) -> Self {
        Self {
            byte_cap,
            ..Self::default()
        }
    }

    pub fn ack_through(&mut self, ack_nr: SeqNr) -> Vec<SentPacket> {
        let keys = self
            .packets
            .keys()
            .copied()
            .filter(|seq| seq.forward_distance_to(ack_nr) < 0x8000)
            .collect::<Vec<_>>();
        self.remove_keys(&keys)
    }

    pub fn sack(&mut self, sack: &SelectiveAck) -> Vec<SentPacket> {
        self.remove_keys(sack.acked())
    }

    pub fn due(&mut self, now: Instant) -> Vec<UtpPacket> {
        let mut packets = Vec::new();
        for sent in self
            .packets
            .values_mut()
            .filter(|sent| now >= sent.deadline)
        {
            sent.retransmissions += 1;
            let backoff = 1u32
                .checked_shl(sent.retransmissions.min(16))
                .unwrap_or(u32::MAX);
            let interval = now.duration_since(sent.sent_at) * backoff;
            sent.deadline = now + interval;
            packets.push(sent.packet.clone());
        }
        packets
    }

    #[must_use]
    pub const fn bytes_in_flight(&self) -> usize {
        self.bytes_in_flight
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.packets.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    fn remove_keys(&mut self, keys: &[SeqNr]) -> Vec<SentPacket> {
        let mut removed = Vec::new();
        for key in keys {
            if let Some(packet) = self.packets.remove(key) {
                self.bytes_in_flight = self
                    .bytes_in_flight
                    .saturating_sub(packet.packet.payload().len());
                removed.push(packet);
            }
        }
        removed
    }
}

impl Default for RetransmitQueue {
    fn default() -> Self {
        Self {
            packets: BTreeMap::new(),
            bytes_in_flight: 0,
            byte_cap: usize::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use bytes::Bytes;

    use crate::{ConnectionId, PacketType, TimestampMicros, WindowBytes};

    use super::*;

    fn packet(seq: u16) -> UtpPacket {
        UtpPacket::new(
            PacketType::Data,
            ConnectionId::new(1),
            TimestampMicros::new(0),
            TimestampMicros::new(0),
            WindowBytes::new(0),
            SeqNr::new(seq),
            SeqNr::new(0),
            Vec::new(),
            Bytes::from_static(b"abcd"),
        )
    }

    #[test]
    fn unacked_data_remains_queued() {
        let now = Instant::now();
        let mut queue = RetransmitQueue::new();
        queue.push(packet(1), now, now + Duration::from_secs(1));

        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn ack_removes_packets_up_to_ack_nr() {
        let now = Instant::now();
        let mut queue = RetransmitQueue::new();
        queue.push(packet(1), now, now + Duration::from_secs(1));
        queue.push(packet(2), now, now + Duration::from_secs(1));

        let removed = queue.ack_through(SeqNr::new(1));

        assert_eq!(removed.len(), 1);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn sack_removes_selectively_acked_packets() {
        let now = Instant::now();
        let mut queue = RetransmitQueue::new();
        queue.push(packet(12), now, now + Duration::from_secs(1));

        let sack = SelectiveAck::parse(SeqNr::new(10), &[1, 0, 0, 0]).unwrap();
        let removed = queue.sack(&sack);

        assert_eq!(removed[0].packet.seq_nr(), SeqNr::new(12));
    }

    #[test]
    fn timeout_returns_due_packets() {
        let now = Instant::now();
        let mut queue = RetransmitQueue::new();
        queue.push(packet(1), now, now + Duration::from_secs(1));

        let due = queue.due(now + Duration::from_secs(2));

        assert_eq!(due[0].seq_nr(), SeqNr::new(1));
    }
}
