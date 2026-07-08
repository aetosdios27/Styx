use std::collections::BTreeSet;

use rand::seq::IteratorRandom;
use rand_chacha::ChaCha8Rng;
use styx_disk::{BlockLength, BlockOffset, PieceIndex};

use crate::{BlockRequest, PeerSession};

#[derive(Clone, Debug)]
pub struct TorrentState {
    piece_count: usize,
    piece_length: u32,
    block_length: u32,
    verified: BTreeSet<PieceIndex>,
    completed_blocks: BTreeSet<BlockRequest>,
}

#[derive(Clone, Debug)]
pub struct PiecePicker {
    startup_random_remaining: usize,
}

impl TorrentState {
    #[must_use]
    pub fn new(piece_count: usize, piece_length: u32, block_length: u32) -> Self {
        Self {
            piece_count,
            piece_length,
            block_length,
            verified: BTreeSet::new(),
            completed_blocks: BTreeSet::new(),
        }
    }

    #[must_use]
    pub const fn piece_count(&self) -> usize {
        self.piece_count
    }

    pub fn block_request(&self, index: u32, begin: u32, length: u32) -> Option<BlockRequest> {
        if index as usize >= self.piece_count {
            return None;
        }
        let length = BlockLength::new(length).ok()?;
        let end = begin.checked_add(length.get())?;
        if begin >= self.piece_length || end > self.piece_length {
            return None;
        }
        Some(BlockRequest::new(
            PieceIndex::new(index),
            BlockOffset::new(begin),
            length,
        ))
    }

    pub fn mark_verified(&mut self, piece: PieceIndex) {
        self.verified.insert(piece);
    }

    pub fn mark_block_complete(&mut self, request: BlockRequest) {
        self.completed_blocks.insert(request);
    }

    #[must_use]
    pub fn is_verified(&self, piece: PieceIndex) -> bool {
        self.verified.contains(&piece)
    }

    #[must_use]
    pub fn is_partial(&self, piece: PieceIndex) -> bool {
        self.completed_blocks
            .iter()
            .any(|request| request.piece == piece)
    }

    fn first_missing_block(
        &self,
        piece: PieceIndex,
        pending: &BTreeSet<BlockRequest>,
    ) -> Option<BlockRequest> {
        let mut offset = 0;
        while offset < self.piece_length {
            let remaining = self.piece_length - offset;
            let length = remaining.min(self.block_length);
            let length = BlockLength::new(length).ok()?;
            let request = BlockRequest::new(piece, BlockOffset::new(offset), length);
            if !self.completed_blocks.contains(&request) && !pending.contains(&request) {
                return Some(request);
            }
            offset += length.get();
        }
        None
    }

    #[must_use]
    pub fn missing_blocks(&self) -> BTreeSet<BlockRequest> {
        let mut missing = BTreeSet::new();
        for piece in 0..self.piece_count {
            let piece = PieceIndex::new(piece as u32);
            if self.is_verified(piece) {
                continue;
            }
            let mut offset = 0;
            while offset < self.piece_length {
                let remaining = self.piece_length - offset;
                let length = remaining.min(self.block_length);
                let Ok(length) = BlockLength::new(length) else {
                    break;
                };
                let request = BlockRequest::new(piece, BlockOffset::new(offset), length);
                if !self.completed_blocks.contains(&request) {
                    missing.insert(request);
                }
                offset += length.get();
            }
        }
        missing
    }
}

impl PiecePicker {
    #[must_use]
    pub const fn new(startup_random_pieces: usize) -> Self {
        Self {
            startup_random_remaining: startup_random_pieces,
        }
    }

    pub fn next_request(
        &mut self,
        peer: &PeerSession,
        torrent: &TorrentState,
        peers: &[PeerSession],
        pending: &BTreeSet<BlockRequest>,
        rng: &mut ChaCha8Rng,
    ) -> Option<BlockRequest> {
        let candidates = peer
            .available_pieces()
            .filter(|piece| !torrent.is_verified(*piece))
            .filter_map(|piece| {
                let availability = peers
                    .iter()
                    .filter(|session| session.has_piece(piece))
                    .count();
                torrent
                    .first_missing_block(piece, pending)
                    .map(|request| Candidate {
                        piece,
                        request,
                        availability,
                        partial: torrent.is_partial(piece),
                    })
            })
            .collect::<Vec<_>>();

        if candidates.is_empty() {
            return None;
        }

        if self.startup_random_remaining > 0 {
            self.startup_random_remaining -= 1;
            return candidates
                .iter()
                .choose(rng)
                .map(|candidate| candidate.request);
        }

        candidates
            .into_iter()
            .min_by(|left, right| {
                left.availability
                    .cmp(&right.availability)
                    .then_with(|| right.partial.cmp(&left.partial))
                    .then_with(|| left.piece.cmp(&right.piece))
                    .then_with(|| left.request.offset.cmp(&right.request.offset))
            })
            .map(|candidate| candidate.request)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Candidate {
    piece: PieceIndex,
    request: BlockRequest,
    availability: usize,
    partial: bool,
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, time::Instant};

    use rand::SeedableRng;
    use styx_disk::{BlockLength, BlockOffset};
    use styx_proto::PeerMessage;

    use crate::PeerKey;

    use super::*;

    fn request(piece: u32, offset: u32) -> BlockRequest {
        BlockRequest::new(
            PieceIndex::new(piece),
            BlockOffset::new(offset),
            BlockLength::new(16).unwrap(),
        )
    }

    fn peer(key: u64, pieces: &[u32]) -> PeerSession {
        let now = Instant::now();
        let mut peer = PeerSession::new(PeerKey::new(key), now, 4).unwrap();
        let mut byte = 0u8;
        for piece in pieces {
            byte |= 0b1000_0000 >> *piece;
        }
        peer.apply_message(
            PeerMessage::Bitfield {
                bytes: bytes::Bytes::from(vec![byte]),
            },
            now,
            4,
        )
        .unwrap();
        peer
    }

    #[test]
    fn next_request_selects_rarest_available_piece() {
        let target = peer(1, &[0, 1]);
        let peers = vec![target.clone(), peer(2, &[0]), peer(3, &[0])];
        let torrent = TorrentState::new(4, 16, 16);
        let mut picker = PiecePicker::new(0);
        let mut rng = ChaCha8Rng::seed_from_u64(1);

        let selected = picker
            .next_request(&target, &torrent, &peers, &BTreeSet::new(), &mut rng)
            .unwrap();

        assert_eq!(selected.piece, PieceIndex::new(1));
    }

    #[test]
    fn next_request_never_selects_verified_piece() {
        let target = peer(1, &[0, 1]);
        let peers = vec![target.clone()];
        let mut torrent = TorrentState::new(4, 16, 16);
        torrent.mark_verified(PieceIndex::new(0));
        let mut picker = PiecePicker::new(0);
        let mut rng = ChaCha8Rng::seed_from_u64(1);

        let selected = picker
            .next_request(&target, &torrent, &peers, &BTreeSet::new(), &mut rng)
            .unwrap();

        assert_eq!(selected.piece, PieceIndex::new(1));
    }

    #[test]
    fn next_request_skips_pending_blocks_before_endgame() {
        let target = peer(1, &[0]);
        let peers = vec![target.clone()];
        let torrent = TorrentState::new(4, 32, 16);
        let mut pending = BTreeSet::new();
        pending.insert(request(0, 0));
        let mut picker = PiecePicker::new(0);
        let mut rng = ChaCha8Rng::seed_from_u64(1);

        let selected = picker
            .next_request(&target, &torrent, &peers, &pending, &mut rng)
            .unwrap();

        assert_eq!(selected.offset, BlockOffset::new(16));
    }

    #[test]
    fn next_request_prefers_partial_piece_when_availability_ties() {
        let target = peer(1, &[0, 1]);
        let peers = vec![target.clone()];
        let mut torrent = TorrentState::new(4, 32, 16);
        torrent.mark_block_complete(request(1, 0));
        let mut picker = PiecePicker::new(0);
        let mut rng = ChaCha8Rng::seed_from_u64(1);

        let selected = picker
            .next_request(&target, &torrent, &peers, &BTreeSet::new(), &mut rng)
            .unwrap();

        assert_eq!(selected.piece, PieceIndex::new(1));
    }

    #[test]
    fn startup_mode_selects_from_available_candidates_under_seeded_rng() {
        let target = peer(1, &[0, 1, 2]);
        let peers = vec![target.clone()];
        let torrent = TorrentState::new(4, 16, 16);
        let mut picker = PiecePicker::new(1);
        let mut rng = ChaCha8Rng::seed_from_u64(2);

        let selected = picker
            .next_request(&target, &torrent, &peers, &BTreeSet::new(), &mut rng)
            .unwrap();

        assert!(matches!(selected.piece.get(), 0..=2));
    }

    #[test]
    fn tie_break_is_stable_after_startup_mode() {
        let target = peer(1, &[0, 1]);
        let peers = vec![target.clone()];
        let torrent = TorrentState::new(4, 16, 16);
        let mut picker = PiecePicker::new(0);
        let mut rng = ChaCha8Rng::seed_from_u64(1);

        let selected = picker
            .next_request(&target, &torrent, &peers, &BTreeSet::new(), &mut rng)
            .unwrap();

        assert_eq!(selected.piece, PieceIndex::new(0));
    }

    #[test]
    fn rarest_first_matches_small_swarm_baseline_shape() {
        let target = peer(1, &[0, 1, 2]);
        let peers = vec![
            target.clone(),
            peer(2, &[0, 1]),
            peer(3, &[0]),
            peer(4, &[0]),
        ];
        let torrent = TorrentState::new(4, 16, 16);
        let mut picker = PiecePicker::new(0);
        let mut rng = ChaCha8Rng::seed_from_u64(1);

        let selected = picker
            .next_request(&target, &torrent, &peers, &BTreeSet::new(), &mut rng)
            .unwrap();

        assert_eq!(selected.piece, PieceIndex::new(2));
    }
}
