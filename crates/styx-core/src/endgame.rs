use std::collections::{BTreeMap, BTreeSet};

use styx_proto::PeerMessage;

use crate::{BlockRequest, PeerAction, PeerKey, PeerSession};

#[derive(Clone, Debug, Default)]
pub struct EndgameController {
    duplicates: BTreeMap<BlockRequest, BTreeSet<PeerKey>>,
}

impl EndgameController {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_active(
        &self,
        missing: &BTreeSet<BlockRequest>,
        assigned: &BTreeMap<BlockRequest, BTreeSet<PeerKey>>,
    ) -> bool {
        !missing.is_empty() && missing.iter().all(|request| assigned.contains_key(request))
    }

    pub fn duplicate_requests(
        &mut self,
        missing: &BTreeSet<BlockRequest>,
        assigned: &BTreeMap<BlockRequest, BTreeSet<PeerKey>>,
        peers: &[PeerSession],
    ) -> Vec<PeerAction> {
        if !self.is_active(missing, assigned) {
            return Vec::new();
        }

        let mut actions = Vec::new();
        for request in missing {
            let Some(existing) = assigned.get(request) else {
                continue;
            };
            for peer in peers {
                if existing.contains(&peer.key()) {
                    continue;
                }
                if self
                    .duplicates
                    .get(request)
                    .is_some_and(|duplicates| duplicates.contains(&peer.key()))
                {
                    continue;
                }
                if !peer.has_piece(request.piece) {
                    continue;
                }
                self.duplicates
                    .entry(*request)
                    .or_default()
                    .insert(peer.key());
                actions.push(PeerAction::SendMessage {
                    peer: peer.key(),
                    message: PeerMessage::Request {
                        index: request.piece.get(),
                        begin: request.offset.get(),
                        length: request.length.get(),
                    },
                });
                break;
            }
        }
        actions
    }

    pub fn complete(&mut self, request: BlockRequest) -> Vec<PeerAction> {
        self.duplicates
            .remove(&request)
            .into_iter()
            .flatten()
            .map(|peer| PeerAction::SendMessage {
                peer,
                message: PeerMessage::Cancel {
                    index: request.piece.get(),
                    begin: request.offset.get(),
                    length: request.length.get(),
                },
            })
            .collect()
    }

    pub fn verification_failed(&mut self, request: BlockRequest) {
        self.duplicates.remove(&request);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        time::Instant,
    };

    use bytes::Bytes;
    use styx_disk::{BlockLength, BlockOffset, PieceIndex};
    use styx_proto::PeerMessage;

    use super::*;

    fn request(piece: u32) -> BlockRequest {
        BlockRequest::new(
            PieceIndex::new(piece),
            BlockOffset::new(0),
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
                bytes: Bytes::from(vec![byte]),
            },
            now,
            4,
        )
        .unwrap();
        peer
    }

    #[test]
    fn is_active_returns_false_while_missing_blocks_are_unrequested() {
        let endgame = EndgameController::new();
        let missing = BTreeSet::from([request(0), request(1)]);
        let assigned = BTreeMap::from([(request(0), BTreeSet::from([PeerKey::new(1)]))]);

        assert!(!endgame.is_active(&missing, &assigned));
    }

    #[test]
    fn duplicate_requests_sends_pending_blocks_to_additional_peers() {
        let mut endgame = EndgameController::new();
        let missing = BTreeSet::from([request(0)]);
        let assigned = BTreeMap::from([(request(0), BTreeSet::from([PeerKey::new(1)]))]);
        let peers = vec![peer(1, &[0]), peer(2, &[0])];

        let actions = endgame.duplicate_requests(&missing, &assigned, &peers);

        assert_eq!(
            actions,
            vec![PeerAction::SendMessage {
                peer: PeerKey::new(2),
                message: PeerMessage::Request {
                    index: 0,
                    begin: 0,
                    length: 16
                }
            }]
        );
    }

    #[test]
    fn duplicate_requests_do_not_duplicate_same_block_to_same_peer_twice() {
        let mut endgame = EndgameController::new();
        let missing = BTreeSet::from([request(0)]);
        let assigned = BTreeMap::from([(request(0), BTreeSet::from([PeerKey::new(1)]))]);
        let peers = vec![peer(1, &[0]), peer(2, &[0])];
        let _ = endgame.duplicate_requests(&missing, &assigned, &peers);

        let actions = endgame.duplicate_requests(&missing, &assigned, &peers);

        assert!(actions.is_empty());
    }

    #[test]
    fn complete_emits_cancel_to_duplicate_peers() {
        let mut endgame = EndgameController::new();
        let missing = BTreeSet::from([request(0)]);
        let assigned = BTreeMap::from([(request(0), BTreeSet::from([PeerKey::new(1)]))]);
        let peers = vec![peer(1, &[0]), peer(2, &[0])];
        let _ = endgame.duplicate_requests(&missing, &assigned, &peers);

        let actions = endgame.complete(request(0));

        assert_eq!(
            actions,
            vec![PeerAction::SendMessage {
                peer: PeerKey::new(2),
                message: PeerMessage::Cancel {
                    index: 0,
                    begin: 0,
                    length: 16
                }
            }]
        );
    }

    #[test]
    fn verification_failed_clears_duplicate_state_without_completion() {
        let mut endgame = EndgameController::new();
        let missing = BTreeSet::from([request(0)]);
        let assigned = BTreeMap::from([(request(0), BTreeSet::from([PeerKey::new(1)]))]);
        let peers = vec![peer(1, &[0]), peer(2, &[0])];
        let _ = endgame.duplicate_requests(&missing, &assigned, &peers);
        endgame.verification_failed(request(0));

        let actions = endgame.complete(request(0));

        assert!(actions.is_empty());
    }
}
