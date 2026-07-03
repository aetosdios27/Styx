use std::net::{Ipv4Addr, Ipv6Addr};

use rand::{CryptoRng, Error as RandError, RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use styx_dht::{
    is_bep42_ipv4_id, is_bep42_ipv6_id, DhtIdentityAction, DhtIdentityManager, ExternalIp,
};

#[test]
fn identity_without_external_ip_is_random_and_session_local() {
    let mut rng = ChaCha8Rng::seed_from_u64(20);
    let mut manager = DhtIdentityManager::new();

    let first = manager.rotate_without_external_ip(&mut rng).unwrap();
    let second = manager.rotate_without_external_ip(&mut rng).unwrap();

    assert_ne!(first.node_id, second.node_id);
}

#[test]
fn identity_with_ipv4_external_ip_satisfies_bep42() {
    let mut rng = ChaCha8Rng::seed_from_u64(21);
    let mut manager = DhtIdentityManager::new();
    let ip = Ipv4Addr::new(124, 31, 75, 21);

    let action = manager
        .observe_external_ip(ExternalIp::V4(ip), &mut rng)
        .unwrap()
        .unwrap();

    assert!(matches!(
        action,
        DhtIdentityAction::RestartWithNodeId { identity }
            if is_bep42_ipv4_id(ip, identity.node_id.as_bytes())
    ));
}

#[test]
fn identity_with_ipv6_external_ip_satisfies_bep42() {
    let mut rng = ChaCha8Rng::seed_from_u64(22);
    let mut manager = DhtIdentityManager::new();
    let ip = Ipv6Addr::new(0x2001, 0x0db8, 0x1234, 0x5678, 0, 0, 0, 1);

    let action = manager
        .observe_external_ip(ExternalIp::V6(ip), &mut rng)
        .unwrap()
        .unwrap();

    assert!(matches!(
        action,
        DhtIdentityAction::RestartWithNodeId { identity }
            if is_bep42_ipv6_id(ip, identity.node_id.as_bytes())
    ));
}

#[test]
fn observing_same_external_ip_returns_none_when_current_id_is_valid() {
    let mut rng = ChaCha8Rng::seed_from_u64(23);
    let mut manager = DhtIdentityManager::new();
    let ip = Ipv4Addr::new(21, 75, 31, 124);

    manager
        .observe_external_ip(ExternalIp::V4(ip), &mut rng)
        .unwrap();
    let action = manager
        .observe_external_ip(ExternalIp::V4(ip), &mut rng)
        .unwrap();

    assert_eq!(action, None);
}

#[test]
fn observing_changed_external_ip_requests_restart() {
    let mut rng = ChaCha8Rng::seed_from_u64(24);
    let mut manager = DhtIdentityManager::new();

    manager
        .observe_external_ip(ExternalIp::V4(Ipv4Addr::new(65, 23, 51, 170)), &mut rng)
        .unwrap();
    let action = manager
        .observe_external_ip(ExternalIp::V4(Ipv4Addr::new(84, 124, 73, 14)), &mut rng)
        .unwrap();

    assert!(action.is_some());
}

#[test]
fn identity_generation_exhausts_after_repeated_node_id_collision() {
    let mut rng = FixedRng([3; 32]);
    let mut manager = DhtIdentityManager::with_max_generation_attempts(1).unwrap();

    manager.rotate_without_external_ip(&mut rng).unwrap();
    let err = manager.rotate_without_external_ip(&mut rng).unwrap_err();

    assert_eq!(
        err.to_string(),
        "could not generate a unique DHT node id after 1 attempts"
    );
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

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), RandError> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for FixedRng {}
