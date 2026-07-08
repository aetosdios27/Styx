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
    pending_uploads: BTreeMap<PeerKey, BTreeSet<BlockRequest>>,
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
            pending_uploads: BTreeMap::new(),
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
        self.pending_uploads.remove(&peer);
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

        match message {
            PeerMessage::Request {
                index,
                begin,
                length,
            } => return self.handle_upload_request(peer, index, begin, length),
            PeerMessage::Cancel {
                index,
                begin,
                length,
            } => return self.handle_upload_cancel(peer, index, begin, length),
            _ => {}
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

    pub fn tick_seed(&mut self, now: Instant) -> Result<Vec<PeerAction>, CoreError> {
        let mut actions = Vec::new();
        let mut peer_list = self.peers.values().cloned().collect::<Vec<_>>();
        actions.extend(self.choke.recalculate(
            &mut peer_list,
            TransferMode::Seeding,
            now,
            &mut self.rng,
        ));
        for peer in peer_list {
            if let Some(stored) = self.peers.get_mut(&peer.key()) {
                stored.set_we_choke(peer.are_we_choking());
            }
        }
        actions.extend(self.drain_ready_uploads());
        Ok(actions)
    }

    pub fn record_uploaded(
        &mut self,
        peer: PeerKey,
        bytes: u64,
        now: Instant,
    ) -> Result<(), CoreError> {
        let session = self
            .peers
            .get_mut(&peer)
            .ok_or(CoreError::UnknownPeer { peer })?;
        session.record_upload(now, bytes);
        Ok(())
    }

    #[must_use]
    pub fn seed_count(&self) -> usize {
        let total = self.torrent.piece_count();
        self.peers
            .values()
            .filter(|s| s.available_pieces().count() == total)
            .count()
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

    fn handle_upload_request(
        &mut self,
        peer: PeerKey,
        index: u32,
        begin: u32,
        length: u32,
    ) -> Result<Vec<PeerAction>, CoreError> {
        let request = self.torrent.block_request(index, begin, length).ok_or(
            CoreError::InvalidPeerMessage {
                reason: "request block is outside torrent bounds",
            },
        )?;
        let session = self
            .peers
            .get(&peer)
            .ok_or(CoreError::UnknownPeer { peer })?;
        if session.are_we_choking() {
            return Ok(Vec::new());
        }
        self.pending_uploads
            .entry(peer)
            .or_default()
            .insert(request);
        Ok(Vec::new())
    }

    fn handle_upload_cancel(
        &mut self,
        peer: PeerKey,
        index: u32,
        begin: u32,
        length: u32,
    ) -> Result<Vec<PeerAction>, CoreError> {
        let request = self.torrent.block_request(index, begin, length).ok_or(
            CoreError::InvalidPeerMessage {
                reason: "cancel block is outside torrent bounds",
            },
        )?;
        let uploads = self
            .pending_uploads
            .get_mut(&peer)
            .ok_or(CoreError::UnknownPeer { peer })?;
        uploads.remove(&request);
        if uploads.is_empty() {
            self.pending_uploads.remove(&peer);
        }
        Ok(Vec::new())
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

    fn drain_ready_uploads(&mut self) -> Vec<PeerAction> {
        let mut actions = Vec::new();
        let peers = self.pending_uploads.keys().copied().collect::<Vec<_>>();
        for peer in peers {
            let Some(session) = self.peers.get(&peer) else {
                self.pending_uploads.remove(&peer);
                continue;
            };
            if session.are_we_choking() {
                continue;
            }
            if let Some(requests) = self.pending_uploads.remove(&peer) {
                actions.extend(
                    requests
                        .into_iter()
                        .map(|request| PeerAction::ServeBlock { peer, request }),
                );
            }
        }
        actions
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

    fn manager_with_config(config: PeerManagerConfig) -> PeerConnectionManager {
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
    fn seed_count_returns_zero_when_no_peers() {
        let manager = manager();
        assert_eq!(manager.seed_count(), 0);
    }

    #[test]
    fn seed_count_reflects_peer_with_full_bitfield() {
        let mut manager = manager();
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();
        // 4 pieces → full bitfield is 0xF0 (first nibble all 1s)
        manager
            .handle_message(
                PeerKey::new(1),
                PeerMessage::Bitfield {
                    bytes: Bytes::from_static(&[0b1111_0000]),
                },
                now,
            )
            .unwrap();
        assert_eq!(manager.seed_count(), 1);
    }

    #[test]
    fn seed_count_excludes_peers_with_partial_bitfield() {
        let mut manager = manager();
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();
        // Only 1 of 4 pieces
        manager
            .handle_message(
                PeerKey::new(1),
                PeerMessage::Bitfield {
                    bytes: Bytes::from_static(&[0b1000_0000]),
                },
                now,
            )
            .unwrap();
        assert_eq!(manager.seed_count(), 0);
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

    #[test]
    fn seed_tick_unchokes_interested_peers_up_to_upload_slots() {
        let config = PeerManagerConfig {
            upload_slots: 1,
            startup_random_pieces: 0,
            ..PeerManagerConfig::default()
        };
        let mut manager = manager_with_config(config);
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();
        manager.add_peer(PeerKey::new(2)).unwrap();
        manager
            .handle_message(PeerKey::new(1), PeerMessage::Interested, now)
            .unwrap();
        manager
            .handle_message(PeerKey::new(2), PeerMessage::Interested, now)
            .unwrap();

        let actions = manager.tick_seed(now).unwrap();

        assert_eq!(
            actions
                .iter()
                .filter(|action| matches!(
                    action,
                    PeerAction::SendMessage {
                        message: PeerMessage::Unchoke,
                        ..
                    }
                ))
                .count(),
            1
        );
    }

    #[test]
    fn seed_tick_chokes_uninterested_peer_without_consuming_slot() {
        let config = PeerManagerConfig {
            upload_slots: 1,
            startup_random_pieces: 0,
            ..PeerManagerConfig::default()
        };
        let mut manager = manager_with_config(config);
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();
        manager.add_peer(PeerKey::new(2)).unwrap();
        manager
            .handle_message(PeerKey::new(1), PeerMessage::Interested, now)
            .unwrap();
        manager
            .handle_message(PeerKey::new(2), PeerMessage::Interested, now)
            .unwrap();
        let _ = manager.tick_seed(now).unwrap();
        manager
            .handle_message(PeerKey::new(1), PeerMessage::NotInterested, now)
            .unwrap();

        let actions = manager.tick_seed(now + config.choke_interval).unwrap();

        assert!(actions.contains(&PeerAction::SendMessage {
            peer: PeerKey::new(1),
            message: PeerMessage::Choke,
        }));
    }

    #[test]
    fn request_from_choked_peer_is_ignored_without_serving() {
        let mut manager = manager();
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();

        let actions = manager
            .handle_message(
                PeerKey::new(1),
                PeerMessage::Request {
                    index: 0,
                    begin: 0,
                    length: 16,
                },
                now,
            )
            .unwrap();

        assert!(!actions
            .iter()
            .any(|action| matches!(action, PeerAction::ServeBlock { .. })));
    }

    #[test]
    fn request_from_unchoked_peer_emits_serve_block_action() {
        let mut manager = manager();
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();
        manager
            .handle_message(PeerKey::new(1), PeerMessage::Interested, now)
            .unwrap();
        let _ = manager.tick_seed(now).unwrap();
        manager
            .handle_message(
                PeerKey::new(1),
                PeerMessage::Request {
                    index: 0,
                    begin: 0,
                    length: 16,
                },
                now,
            )
            .unwrap();

        let actions = manager.tick_seed(now).unwrap();

        assert!(actions.contains(&PeerAction::ServeBlock {
            peer: PeerKey::new(1),
            request: BlockRequest::new(
                PieceIndex::new(0),
                BlockOffset::new(0),
                BlockLength::new(16).unwrap()
            ),
        }));
    }

    #[test]
    fn cancel_removes_pending_upload_request_before_serving() {
        let mut manager = manager();
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();
        manager
            .handle_message(PeerKey::new(1), PeerMessage::Interested, now)
            .unwrap();
        let _ = manager.tick_seed(now).unwrap();
        manager
            .handle_message(
                PeerKey::new(1),
                PeerMessage::Request {
                    index: 0,
                    begin: 0,
                    length: 16,
                },
                now,
            )
            .unwrap();
        manager
            .handle_message(
                PeerKey::new(1),
                PeerMessage::Cancel {
                    index: 0,
                    begin: 0,
                    length: 16,
                },
                now,
            )
            .unwrap();

        let actions = manager.tick_seed(now).unwrap();

        assert!(!actions
            .iter()
            .any(|action| matches!(action, PeerAction::ServeBlock { .. })));
    }

    #[test]
    fn record_uploaded_updates_peer_upload_rate_for_seeding_choke_selection() {
        let config = PeerManagerConfig {
            upload_slots: 1,
            startup_random_pieces: 0,
            ..PeerManagerConfig::default()
        };
        let mut manager = manager_with_config(config);
        let now = Instant::now();
        manager.add_peer(PeerKey::new(1)).unwrap();
        manager.add_peer(PeerKey::new(2)).unwrap();
        manager
            .handle_message(PeerKey::new(1), PeerMessage::Interested, now)
            .unwrap();
        manager
            .handle_message(PeerKey::new(2), PeerMessage::Interested, now)
            .unwrap();
        manager.record_uploaded(PeerKey::new(2), 1600, now).unwrap();

        let actions = manager.tick_seed(now).unwrap();

        assert!(actions.contains(&PeerAction::SendMessage {
            peer: PeerKey::new(2),
            message: PeerMessage::Unchoke,
        }));
    }
}
