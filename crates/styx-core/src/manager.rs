use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

use bytes::Bytes;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use styx_disk::{BlockLength, BlockOffset, PieceIndex};
use styx_proto::PeerMessage;

use crate::{
    BlockRequest, ChokeController, CoreError, DisconnectReason, EndgameController, PeerAction,
    PeerKey, PeerManagerConfig, PeerSession, PiecePicker, RequestPipeline, TorrentState,
    TransferMode,
};

#[derive(Debug)]
pub struct PeerConnectionManager {
    config: PeerManagerConfig,
    torrent: TorrentState,
    peers: BTreeMap<PeerKey, PeerSession>,
    pipelines: BTreeMap<PeerKey, RequestPipeline>,
    choke: ChokeController,
    picker: PiecePicker,
    endgame: EndgameController,
    rng: ChaCha8Rng,
}

impl PeerConnectionManager {
    pub fn new(config: PeerManagerConfig, torrent: TorrentState) -> Result<Self, CoreError> {
        let config = config.validate()?;
        let now = Instant::now();
        Ok(Self {
            config,
            torrent,
            peers: BTreeMap::new(),
            pipelines: BTreeMap::new(),
            choke: ChokeController::new(config, now),
            picker: PiecePicker::new(config.startup_random_pieces),
            endgame: EndgameController::new(),
            rng: ChaCha8Rng::seed_from_u64(0x57_59_58),
        })
    }

    pub fn add_peer(&mut self, peer: PeerKey) -> Result<Vec<PeerAction>, CoreError> {
        if self.peers.contains_key(&peer) {
            return Err(CoreError::PeerAlreadyExists { peer });
        }
        let now = Instant::now();
        self.peers.insert(
            peer,
            PeerSession::new(peer, now, self.torrent.piece_count())?,
        );
        self.pipelines.insert(
            peer,
            RequestPipeline::new(peer, self.config.request_pipeline_depth)?,
        );
        Ok(Vec::new())
    }

    pub fn remove_peer(&mut self, peer: PeerKey) -> Result<Vec<PeerAction>, CoreError> {
        if self.peers.remove(&peer).is_none() {
            return Err(CoreError::UnknownPeer { peer });
        }
        self.pipelines.remove(&peer);
        Ok(vec![PeerAction::Disconnect {
            peer,
            reason: DisconnectReason::Removed,
        }])
    }

    pub fn handle_message(
        &mut self,
        peer: PeerKey,
        message: PeerMessage,
        now: Instant,
    ) -> Result<Vec<PeerAction>, CoreError> {
        if let PeerMessage::Piece {
            index,
            begin,
            block,
        } = message
        {
            return self.handle_piece(peer, index, begin, block, now);
        }

        let session = self
            .peers
            .get_mut(&peer)
            .ok_or(CoreError::UnknownPeer { peer })?;
        let mut actions = session.apply_message(message, now, self.torrent.piece_count())?;
        if session.are_we_interested() {
            actions.push(PeerAction::SendMessage {
                peer,
                message: PeerMessage::Interested,
            });
        }
        Ok(actions)
    }

    pub fn tick(&mut self, now: Instant) -> Result<Vec<PeerAction>, CoreError> {
        let mut actions = self.cancel_stalled(now)?;
        let mut peer_list = self.peers.values().cloned().collect::<Vec<_>>();
        actions.extend(self.choke.recalculate(
            &mut peer_list,
            TransferMode::Leeching,
            now,
            &mut self.rng,
        ));
        for peer in peer_list {
            if let Some(stored) = self.peers.get_mut(&peer.key()) {
                stored.set_we_choke(peer.are_we_choking());
            }
        }

        actions.extend(self.request_blocks(now)?);
        actions.extend(self.endgame_actions());
        Ok(actions)
    }

    pub fn mark_piece_verified(&mut self, request: BlockRequest) -> Vec<PeerAction> {
        self.torrent.mark_block_complete(request);
        self.endgame.complete(request)
    }

    fn handle_piece(
        &mut self,
        peer: PeerKey,
        index: u32,
        begin: u32,
        block: Bytes,
        now: Instant,
    ) -> Result<Vec<PeerAction>, CoreError> {
        let length =
            BlockLength::new(block.len() as u32).map_err(|_| CoreError::InvalidPeerMessage {
                reason: "piece block has invalid length",
            })?;
        let request = BlockRequest::new(PieceIndex::new(index), BlockOffset::new(begin), length);
        let pipeline = self
            .pipelines
            .get_mut(&peer)
            .ok_or(CoreError::UnknownPeer { peer })?;
        pipeline.complete(request)?;
        if let Some(session) = self.peers.get_mut(&peer) {
            session.record_download(now, block.len() as u64);
        }
        Ok(vec![PeerAction::AcceptBlock {
            peer,
            request,
            bytes: block,
        }])
    }

    fn request_blocks(&mut self, now: Instant) -> Result<Vec<PeerAction>, CoreError> {
        let mut actions = Vec::new();
        let pending = self.pending_requests();
        let peers = self.peers.values().cloned().collect::<Vec<_>>();
        let mut pending_mut = pending;

        for peer in self.peers.values() {
            if peer.is_peer_choked() || !peer.are_we_interested() {
                continue;
            }
            let Some(pipeline) = self.pipelines.get_mut(&peer.key()) else {
                continue;
            };
            while !pipeline.is_full() {
                let Some(request) = self.picker.next_request(
                    peer,
                    &self.torrent,
                    &peers,
                    &pending_mut,
                    &mut self.rng,
                ) else {
                    break;
                };
                pipeline.add(request, now)?;
                pending_mut.insert(request);
                actions.push(PeerAction::SendMessage {
                    peer: peer.key(),
                    message: PeerMessage::Request {
                        index: request.piece.get(),
                        begin: request.offset.get(),
                        length: request.length.get(),
                    },
                });
            }
        }

        Ok(actions)
    }

    fn cancel_stalled(&mut self, now: Instant) -> Result<Vec<PeerAction>, CoreError> {
        let mut actions = Vec::new();
        for (peer, pipeline) in &mut self.pipelines {
            let stalled = pipeline.stalled(now, self.config.request_timeout);
            for in_flight in stalled {
                pipeline.cancel(in_flight.request)?;
                actions.push(PeerAction::SendMessage {
                    peer: *peer,
                    message: PeerMessage::Cancel {
                        index: in_flight.request.piece.get(),
                        begin: in_flight.request.offset.get(),
                        length: in_flight.request.length.get(),
                    },
                });
            }
        }
        Ok(actions)
    }

    fn pending_requests(&self) -> BTreeSet<BlockRequest> {
        self.pipelines
            .values()
            .flat_map(RequestPipeline::requests)
            .map(|request| request.request)
            .collect()
    }

    fn assigned_requests(&self) -> BTreeMap<BlockRequest, BTreeSet<PeerKey>> {
        let mut assigned: BTreeMap<BlockRequest, BTreeSet<PeerKey>> = BTreeMap::new();
        for request in self.pipelines.values().flat_map(RequestPipeline::requests) {
            assigned
                .entry(request.request)
                .or_default()
                .insert(request.peer);
        }
        assigned
    }

    fn endgame_actions(&mut self) -> Vec<PeerAction> {
        let missing = self.torrent.missing_blocks();
        let assigned = self.assigned_requests();
        let peers = self.peers.values().cloned().collect::<Vec<_>>();
        self.endgame.duplicate_requests(&missing, &assigned, &peers)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use bytes::Bytes;
    use styx_proto::PeerMessage;

    use super::*;

    fn manager() -> PeerConnectionManager {
        let config = PeerManagerConfig {
            startup_random_pieces: 0,
            ..PeerManagerConfig::default()
        };
        PeerConnectionManager::new(config, TorrentState::new(4, 80, 16)).unwrap()
    }

    #[test]
    fn tick_emits_requests_up_to_pipeline_depth_after_peer_advertises_pieces() {
        let mut manager = manager();
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();
        manager
            .handle_message(
                PeerKey::new(1),
                PeerMessage::Bitfield {
                    bytes: Bytes::from_static(&[0b1000_0000]),
                },
                now,
            )
            .unwrap();
        manager
            .handle_message(PeerKey::new(1), PeerMessage::Unchoke, now)
            .unwrap();

        let actions = manager.tick(now);
        let requests = actions
            .unwrap()
            .into_iter()
            .filter(|action| {
                matches!(
                    action,
                    PeerAction::SendMessage {
                        message: PeerMessage::Request { .. },
                        ..
                    }
                )
            })
            .count();

        assert_eq!(requests, 5);
    }

    #[test]
    fn tick_emits_choke_actions_at_controller_intervals() {
        let mut manager = manager();
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();
        manager
            .handle_message(PeerKey::new(1), PeerMessage::Interested, now)
            .unwrap();

        let actions = manager.tick(now).unwrap();

        assert!(actions.contains(&PeerAction::SendMessage {
            peer: PeerKey::new(1),
            message: PeerMessage::Unchoke,
        }));
    }

    #[test]
    fn handle_piece_completes_request_and_emits_accept_block_effect() {
        let mut manager = manager();
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();
        manager
            .handle_message(
                PeerKey::new(1),
                PeerMessage::Bitfield {
                    bytes: Bytes::from_static(&[0b1000_0000]),
                },
                now,
            )
            .unwrap();
        manager
            .handle_message(PeerKey::new(1), PeerMessage::Unchoke, now)
            .unwrap();
        let _ = manager.tick(now).unwrap();

        let actions = manager
            .handle_message(
                PeerKey::new(1),
                PeerMessage::Piece {
                    index: 0,
                    begin: 0,
                    block: Bytes::from_static(&[1; 16]),
                },
                now,
            )
            .unwrap();

        assert!(matches!(
            &actions[0],
            PeerAction::AcceptBlock {
                peer,
                request,
                ..
            } if *peer == PeerKey::new(1) && request.piece == PieceIndex::new(0)
        ));
    }

    #[test]
    fn tick_cancels_stalled_requests() {
        let mut manager = manager();
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();
        manager
            .handle_message(
                PeerKey::new(1),
                PeerMessage::Bitfield {
                    bytes: Bytes::from_static(&[0b1000_0000]),
                },
                now,
            )
            .unwrap();
        manager
            .handle_message(PeerKey::new(1), PeerMessage::Unchoke, now)
            .unwrap();
        let _ = manager.tick(now).unwrap();

        let actions = manager.tick(now + Duration::from_secs(31)).unwrap();

        assert!(actions.iter().any(|action| {
            matches!(
                action,
                PeerAction::SendMessage {
                    message: PeerMessage::Cancel { .. },
                    ..
                }
            )
        }));
    }

    #[test]
    fn handle_message_rejects_unknown_peer() {
        let mut manager = manager();

        let err = manager
            .handle_message(PeerKey::new(99), PeerMessage::KeepAlive, Instant::now())
            .unwrap_err();

        assert_eq!(
            err,
            CoreError::UnknownPeer {
                peer: PeerKey::new(99)
            }
        );
    }
}
