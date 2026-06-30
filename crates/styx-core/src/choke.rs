use std::time::Instant;

use rand::seq::IteratorRandom;
use rand_chacha::ChaCha8Rng;
use styx_proto::PeerMessage;

use crate::{PeerAction, PeerKey, PeerManagerConfig, PeerSession};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferMode {
    Leeching,
    Seeding,
}

#[derive(Debug)]
pub struct ChokeController {
    config: PeerManagerConfig,
    last_regular: Instant,
    last_optimistic: Instant,
}

impl ChokeController {
    #[must_use]
    pub fn new(config: PeerManagerConfig, now: Instant) -> Self {
        Self {
            config,
            last_regular: now - config.choke_interval,
            last_optimistic: now,
        }
    }

    pub fn recalculate(
        &mut self,
        peers: &mut [PeerSession],
        mode: TransferMode,
        now: Instant,
        rng: &mut ChaCha8Rng,
    ) -> Vec<PeerAction> {
        let mut actions = Vec::new();
        let mut selected = Vec::new();

        if now.duration_since(self.last_regular) >= self.config.choke_interval {
            self.last_regular = now;
            selected = regular_slots(peers, mode, now, self.config.upload_slots);
            actions.extend(apply_regular_selection(peers, &selected));
        }

        if now.duration_since(self.last_optimistic) >= self.config.optimistic_unchoke_interval {
            self.last_optimistic = now;
            if let Some(peer) = peers
                .iter_mut()
                .filter(|peer| peer.is_peer_interested())
                .filter(|peer| peer.are_we_choking())
                .filter(|peer| !selected.contains(&peer.key()))
                .choose(rng)
            {
                peer.set_we_choke(false);
                actions.push(PeerAction::SendMessage {
                    peer: peer.key(),
                    message: PeerMessage::Unchoke,
                });
            }
        }

        actions
    }
}

fn regular_slots(
    peers: &mut [PeerSession],
    mode: TransferMode,
    now: Instant,
    upload_slots: usize,
) -> Vec<PeerKey> {
    let mut ranked = peers
        .iter_mut()
        .filter(|peer| peer.is_peer_interested())
        .map(|peer| {
            let rate = match mode {
                TransferMode::Leeching => peer.download_rate(now),
                TransferMode::Seeding => peer.upload_rate(now),
            };
            (peer.key(), rate)
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|(left_peer, left_rate), (right_peer, right_rate)| {
        right_rate
            .cmp(left_rate)
            .then_with(|| left_peer.cmp(right_peer))
    });
    ranked
        .into_iter()
        .take(upload_slots)
        .map(|(peer, _)| peer)
        .collect()
}

fn apply_regular_selection(peers: &mut [PeerSession], selected: &[PeerKey]) -> Vec<PeerAction> {
    let mut actions = Vec::new();
    for peer in peers {
        let should_unchoke = peer.is_peer_interested() && selected.contains(&peer.key());
        if should_unchoke && peer.are_we_choking() {
            peer.set_we_choke(false);
            actions.push(PeerAction::SendMessage {
                peer: peer.key(),
                message: PeerMessage::Unchoke,
            });
        } else if !should_unchoke && !peer.are_we_choking() {
            peer.set_we_choke(true);
            actions.push(PeerAction::SendMessage {
                peer: peer.key(),
                message: PeerMessage::Choke,
            });
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use rand::SeedableRng;

    use super::*;

    fn peer(key: u64, interested: bool, download: u64, upload: u64, now: Instant) -> PeerSession {
        let mut peer = PeerSession::new(PeerKey::new(key), now, 8).unwrap();
        peer.set_peer_interested(interested);
        peer.record_download(now, download * 20);
        peer.record_upload(now, upload * 20);
        peer
    }

    fn unchoked(actions: &[PeerAction]) -> Vec<PeerKey> {
        let mut peers = actions
            .iter()
            .filter_map(|action| match action {
                PeerAction::SendMessage {
                    peer,
                    message: PeerMessage::Unchoke,
                } => Some(*peer),
                _ => None,
            })
            .collect::<Vec<_>>();
        peers.sort_unstable();
        peers
    }

    #[test]
    fn recalculate_unchokes_top_interested_leeching_peers_by_download_rate() {
        let now = Instant::now();
        let mut peers = vec![
            peer(1, true, 10, 1, now),
            peer(2, true, 50, 1, now),
            peer(3, true, 20, 1, now),
            peer(4, true, 40, 1, now),
            peer(5, true, 30, 1, now),
        ];
        let mut controller = ChokeController::new(PeerManagerConfig::default(), now);
        let mut rng = ChaCha8Rng::seed_from_u64(1);

        let actions = controller.recalculate(&mut peers, TransferMode::Leeching, now, &mut rng);

        assert_eq!(
            unchoked(&actions),
            vec![
                PeerKey::new(2),
                PeerKey::new(3),
                PeerKey::new(4),
                PeerKey::new(5)
            ]
        );
    }

    #[test]
    fn recalculate_uses_upload_rate_when_seeding() {
        let now = Instant::now();
        let mut peers = vec![
            peer(1, true, 100, 1, now),
            peer(2, true, 90, 50, now),
            peer(3, true, 80, 20, now),
            peer(4, true, 70, 40, now),
            peer(5, true, 60, 30, now),
        ];
        let mut controller = ChokeController::new(PeerManagerConfig::default(), now);
        let mut rng = ChaCha8Rng::seed_from_u64(1);

        let actions = controller.recalculate(&mut peers, TransferMode::Seeding, now, &mut rng);

        assert_eq!(
            unchoked(&actions),
            vec![
                PeerKey::new(2),
                PeerKey::new(3),
                PeerKey::new(4),
                PeerKey::new(5)
            ]
        );
    }

    #[test]
    fn uninterested_peers_are_choked_and_do_not_consume_slots() {
        let now = Instant::now();
        let mut peers = vec![peer(1, false, 100, 1, now), peer(2, true, 10, 1, now)];
        peers[0].set_we_choke(false);
        let mut controller = ChokeController::new(PeerManagerConfig::default(), now);
        let mut rng = ChaCha8Rng::seed_from_u64(1);

        let actions = controller.recalculate(&mut peers, TransferMode::Leeching, now, &mut rng);

        assert!(actions.contains(&PeerAction::SendMessage {
            peer: PeerKey::new(1),
            message: PeerMessage::Choke,
        }));
        assert!(actions.contains(&PeerAction::SendMessage {
            peer: PeerKey::new(2),
            message: PeerMessage::Unchoke,
        }));
    }

    #[test]
    fn recalculate_before_interval_returns_no_actions() {
        let now = Instant::now();
        let mut peers = vec![peer(1, true, 10, 1, now)];
        let mut controller = ChokeController::new(PeerManagerConfig::default(), now);
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let _ = controller.recalculate(&mut peers, TransferMode::Leeching, now, &mut rng);

        let actions = controller.recalculate(
            &mut peers,
            TransferMode::Leeching,
            now + Duration::from_secs(1),
            &mut rng,
        );

        assert!(actions.is_empty());
    }

    #[test]
    fn optimistic_unchoke_selects_choked_interested_peer_every_thirty_seconds() {
        let now = Instant::now();
        let config = PeerManagerConfig {
            upload_slots: 1,
            ..PeerManagerConfig::default()
        };
        let mut peers = vec![peer(1, true, 100, 1, now), peer(2, true, 1, 1, now)];
        let mut controller = ChokeController::new(config, now);
        let mut rng = ChaCha8Rng::seed_from_u64(1);

        let _ = controller.recalculate(&mut peers, TransferMode::Leeching, now, &mut rng);
        let actions = controller.recalculate(
            &mut peers,
            TransferMode::Leeching,
            now + Duration::from_secs(30),
            &mut rng,
        );

        assert!(unchoked(&actions).contains(&PeerKey::new(2)));
    }

    #[test]
    fn new_interested_peer_is_eligible_for_optimistic_unchoke() {
        let now = Instant::now();
        let config = PeerManagerConfig {
            upload_slots: 1,
            ..PeerManagerConfig::default()
        };
        let mut peers = vec![peer(1, true, 100, 1, now), peer(99, true, 0, 0, now)];
        let mut controller = ChokeController::new(config, now);
        let mut rng = ChaCha8Rng::seed_from_u64(9);

        let _ = controller.recalculate(&mut peers, TransferMode::Leeching, now, &mut rng);
        let actions = controller.recalculate(
            &mut peers,
            TransferMode::Leeching,
            now + Duration::from_secs(30),
            &mut rng,
        );

        assert!(unchoked(&actions).contains(&PeerKey::new(99)));
    }
}
