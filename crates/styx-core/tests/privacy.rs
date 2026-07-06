use std::collections::HashSet;

use rand::{CryptoRng, RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use styx_core::{
    CoreError, PeerIdPrefix, PeerIdentityManager, PeerKey, PrivacyAction, PrivacyConfig,
    PrivacyController, PrivacyIntent,
};

#[test]
fn default_config_generates_unbranded_peer_ids() {
    let mut rng = ChaCha8Rng::seed_from_u64(10);
    let mut manager = PeerIdentityManager::new(PrivacyConfig::default()).unwrap();

    let ids = (0..16)
        .map(|_| manager.generate(&mut rng).unwrap().peer_id)
        .collect::<Vec<_>>();

    assert!(ids
        .iter()
        .all(|peer_id| !peer_id.as_bytes().starts_with(b"-ST")));
}

#[test]
fn peer_id_generation_never_reuses_identity_in_one_manager() {
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let mut manager = PeerIdentityManager::new(PrivacyConfig::default()).unwrap();

    let ids = (0..1_000)
        .map(|_| manager.generate(&mut rng).unwrap().peer_id)
        .collect::<HashSet<_>>();

    assert_eq!(ids.len(), 1_000);
}

#[test]
fn configured_prefix_is_opt_in_and_preserved() {
    let mut rng = ChaCha8Rng::seed_from_u64(12);
    let config = PrivacyConfig {
        peer_id_prefix: Some(PeerIdPrefix::new(*b"-ST0001-")),
        ..PrivacyConfig::default()
    };
    let mut manager = PeerIdentityManager::new(config).unwrap();

    let identity = manager.generate(&mut rng).unwrap();

    assert_eq!(&identity.peer_id.as_bytes()[..8], b"-ST0001-");
}

#[test]
fn announce_identity_rotates_every_time() {
    let mut rng = ChaCha8Rng::seed_from_u64(13);
    let mut controller = PrivacyController::new(PrivacyConfig::default()).unwrap();

    let first = controller
        .apply(PrivacyIntent::NewAnnounceIdentity, &mut rng)
        .unwrap();
    let second = controller
        .apply(PrivacyIntent::NewAnnounceIdentity, &mut rng)
        .unwrap();

    assert_ne!(first, second);
}

#[test]
fn reconnect_identity_disconnects_with_fresh_peer_id() {
    let mut rng = ChaCha8Rng::seed_from_u64(14);
    let mut controller = PrivacyController::new(PrivacyConfig::default()).unwrap();
    let peer = PeerKey::new(99);

    let action = controller
        .apply(PrivacyIntent::ReconnectWithFreshIdentity { peer }, &mut rng)
        .unwrap();

    assert!(matches!(
        action,
        PrivacyAction::DisconnectForPrivacy {
            peer: observed,
            ..
        } if observed == peer
    ));
}

#[test]
fn zero_generation_attempts_is_invalid() {
    let config = PrivacyConfig {
        max_generation_attempts: 0,
        ..PrivacyConfig::default()
    };

    let err = PeerIdentityManager::new(config).unwrap_err();

    assert_eq!(
        err,
        CoreError::InvalidPrivacyConfig {
            field: "max_generation_attempts"
        }
    );
}

#[test]
fn rng_collision_returns_typed_error_after_attempt_limit() {
    let mut rng = FixedRng([7; 32]);
    let config = PrivacyConfig {
        max_generation_attempts: 2,
        ..PrivacyConfig::default()
    };
    let mut manager = PeerIdentityManager::new(config).unwrap();

    manager.generate(&mut rng).unwrap();
    let err = manager.generate(&mut rng).unwrap_err();

    assert_eq!(err, CoreError::PeerIdExhausted { attempts: 2 });
}

#[derive(Debug)]
struct FixedRng([u8; 32]);

impl RngCore for FixedRng {
    fn next_u32(&mut self) -> u32 {
        u32::from_le_bytes([self.0[0], self.0[1], self.0[2], self.0[3]])
    }

    fn next_u64(&mut self) -> u64 {
        u64::from_le_bytes([
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5], self.0[6], self.0[7],
        ])
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for (index, byte) in dest.iter_mut().enumerate() {
            *byte = self.0[index % self.0.len()];
        }
    }
}

impl CryptoRng for FixedRng {}
