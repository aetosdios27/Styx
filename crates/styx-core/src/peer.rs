use std::time::Instant;

use styx_disk::PieceIndex;
use styx_proto::PeerMessage;

use crate::{CoreError, PeerAction, PeerKey, RateWindow};

#[derive(Clone, Debug)]
pub struct PeerSession {
    key: PeerKey,
    peer_choked: bool,
    we_choke: bool,
    peer_interested: bool,
    we_interested: bool,
    available: Vec<bool>,
    download_rate: RateWindow,
    upload_rate: RateWindow,
}

impl PeerSession {
    pub fn new(key: PeerKey, now: Instant, piece_count: usize) -> Result<Self, CoreError> {
        let _ = now;
        Ok(Self {
            key,
            peer_choked: true,
            we_choke: true,
            peer_interested: false,
            we_interested: false,
            available: vec![false; piece_count],
            download_rate: RateWindow::new(crate::PeerManagerConfig::default().rate_window)?,
            upload_rate: RateWindow::new(crate::PeerManagerConfig::default().rate_window)?,
        })
    }

    #[must_use]
    pub const fn key(&self) -> PeerKey {
        self.key
    }

    #[must_use]
    pub const fn is_peer_choked(&self) -> bool {
        self.peer_choked
    }

    #[must_use]
    pub const fn are_we_choking(&self) -> bool {
        self.we_choke
    }

    #[must_use]
    pub const fn is_peer_interested(&self) -> bool {
        self.peer_interested
    }

    #[must_use]
    pub const fn are_we_interested(&self) -> bool {
        self.we_interested
    }

    #[must_use]
    pub fn in_flight_len(&self) -> usize {
        0
    }

    #[must_use]
    pub fn has_piece(&self, piece: PieceIndex) -> bool {
        self.available
            .get(piece.get() as usize)
            .copied()
            .unwrap_or(false)
    }

    #[must_use]
    pub fn has_any_availability(&self) -> bool {
        self.available.iter().any(|available| *available)
    }

    pub fn available_pieces(&self) -> impl Iterator<Item = PieceIndex> + '_ {
        self.available
            .iter()
            .enumerate()
            .filter(|(_, available)| **available)
            .map(|(index, _)| PieceIndex::new(index as u32))
    }

    pub fn set_we_choke(&mut self, choked: bool) {
        self.we_choke = choked;
    }

    pub fn set_peer_interested(&mut self, interested: bool) {
        self.peer_interested = interested;
    }

    pub fn record_download(&mut self, now: Instant, bytes: u64) {
        self.download_rate.record(now, bytes);
    }

    pub fn record_upload(&mut self, now: Instant, bytes: u64) {
        self.upload_rate.record(now, bytes);
    }

    pub fn download_rate(&mut self, now: Instant) -> u64 {
        self.download_rate.bytes_per_second(now)
    }

    pub fn upload_rate(&mut self, now: Instant) -> u64 {
        self.upload_rate.bytes_per_second(now)
    }

    pub fn apply_message(
        &mut self,
        message: PeerMessage,
        _now: Instant,
        piece_count: usize,
    ) -> Result<Vec<PeerAction>, CoreError> {
        match message {
            PeerMessage::Choke => self.peer_choked = true,
            PeerMessage::Unchoke => self.peer_choked = false,
            PeerMessage::Interested => {
                self.peer_interested = true;
                return Ok(vec![PeerAction::RecordInterest {
                    peer: self.key,
                    interested: true,
                }]);
            }
            PeerMessage::NotInterested => {
                self.peer_interested = false;
                return Ok(vec![PeerAction::RecordInterest {
                    peer: self.key,
                    interested: false,
                }]);
            }
            PeerMessage::Have { piece_index } => {
                let index = piece_index as usize;
                if index >= piece_count {
                    return Err(CoreError::InvalidPeerMessage {
                        reason: "have piece index out of range",
                    });
                }
                self.available[index] = true;
                self.we_interested = true;
            }
            PeerMessage::Bitfield { bytes } => {
                self.available = decode_bitfield(&bytes, piece_count)?;
                self.we_interested = self.available.iter().any(|available| *available);
            }
            PeerMessage::KeepAlive
            | PeerMessage::Request { .. }
            | PeerMessage::Piece { .. }
            | PeerMessage::Cancel { .. } => {}
            PeerMessage::HashRequest(_) | PeerMessage::Hashes(_) | PeerMessage::HashReject(_) => {}
        }
        Ok(Vec::new())
    }
}

fn decode_bitfield(bytes: &[u8], piece_count: usize) -> Result<Vec<bool>, CoreError> {
    let expected_len = piece_count.div_ceil(8);
    if bytes.len() != expected_len {
        return Err(CoreError::InvalidBitfieldLength {
            expected_pieces: piece_count,
            actual_bits: bytes.len() * 8,
        });
    }

    let mut available = vec![false; piece_count];
    for piece in 0..piece_count {
        let byte = bytes[piece / 8];
        let mask = 0b1000_0000 >> (piece % 8);
        available[piece] = byte & mask != 0;
    }

    let spare_bits = expected_len * 8 - piece_count;
    if spare_bits > 0 {
        let spare_mask = (1u8 << spare_bits) - 1;
        if bytes.last().is_some_and(|last| last & spare_mask != 0) {
            return Err(CoreError::InvalidBitfieldLength {
                expected_pieces: piece_count,
                actual_bits: bytes.len() * 8,
            });
        }
    }

    Ok(available)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use bytes::Bytes;
    use styx_disk::PieceIndex;
    use styx_proto::PeerMessage;

    use super::*;

    fn session() -> PeerSession {
        PeerSession::new(PeerKey::new(7), Instant::now(), 8).unwrap()
    }

    #[test]
    fn new_starts_with_conservative_peer_state() {
        let peer = session();

        assert!(peer.is_peer_choked());
        assert!(peer.are_we_choking());
        assert!(!peer.is_peer_interested());
        assert!(!peer.has_any_availability());
        assert_eq!(peer.in_flight_len(), 0);
    }

    #[test]
    fn apply_message_records_bitfield_availability() {
        let mut peer = session();

        peer.apply_message(
            PeerMessage::Bitfield {
                bytes: Bytes::from_static(&[0b1010_0000]),
            },
            Instant::now(),
            8,
        )
        .unwrap();

        assert!(peer.has_piece(PieceIndex::new(0)));
        assert!(peer.has_piece(PieceIndex::new(2)));
        assert!(!peer.has_piece(PieceIndex::new(1)));
    }

    #[test]
    fn apply_message_records_have_availability() {
        let mut peer = session();

        peer.apply_message(PeerMessage::Have { piece_index: 3 }, Instant::now(), 8)
            .unwrap();

        assert!(peer.has_piece(PieceIndex::new(3)));
    }

    #[test]
    fn apply_message_records_choke_state() {
        let mut peer = session();

        peer.apply_message(PeerMessage::Unchoke, Instant::now(), 8)
            .unwrap();

        assert!(!peer.is_peer_choked());
    }

    #[test]
    fn apply_message_records_interest_actions() {
        let mut peer = session();

        let actions = peer
            .apply_message(PeerMessage::Interested, Instant::now(), 8)
            .unwrap();

        assert_eq!(
            actions,
            vec![PeerAction::RecordInterest {
                peer: PeerKey::new(7),
                interested: true
            }]
        );
    }

    #[test]
    fn apply_message_rejects_bitfield_with_too_many_bytes() {
        let mut peer = session();

        let err = peer
            .apply_message(
                PeerMessage::Bitfield {
                    bytes: Bytes::from_static(&[0, 0]),
                },
                Instant::now(),
                8,
            )
            .unwrap_err();

        assert_eq!(
            err,
            CoreError::InvalidBitfieldLength {
                expected_pieces: 8,
                actual_bits: 16
            }
        );
    }

    #[test]
    fn apply_message_rejects_have_outside_piece_count() {
        let mut peer = session();

        let err = peer
            .apply_message(PeerMessage::Have { piece_index: 8 }, Instant::now(), 8)
            .unwrap_err();

        assert_eq!(
            err,
            CoreError::InvalidPeerMessage {
                reason: "have piece index out of range"
            }
        );
    }
}
