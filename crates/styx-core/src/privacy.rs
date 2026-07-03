use std::collections::HashSet;

use rand::{CryptoRng, RngCore};
use styx_proto::PeerId;

use crate::{CoreError, PeerKey};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IdentityEpoch(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerIdentity {
    pub epoch: IdentityEpoch,
    pub peer_id: PeerId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerIdPrefix([u8; 8]);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivacyConfig {
    pub peer_id_prefix: Option<PeerIdPrefix>,
    pub max_generation_attempts: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivacyIntent {
    NewAnnounceIdentity,
    ReconnectWithFreshIdentity { peer: PeerKey },
    RotatePeerIdentity { peer: PeerKey },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrivacyAction {
    UsePeerId {
        identity: PeerIdentity,
    },
    DisconnectForPrivacy {
        peer: PeerKey,
        identity: PeerIdentity,
    },
}

#[derive(Debug)]
pub struct PeerIdentityManager {
    config: PrivacyConfig,
    generated: HashSet<PeerId>,
    next_epoch: u64,
}

#[derive(Debug)]
pub struct PrivacyController {
    identities: PeerIdentityManager,
}

impl IdentityEpoch {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl PeerIdPrefix {
    #[must_use]
    pub const fn new(value: [u8; 8]) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            peer_id_prefix: None,
            max_generation_attempts: 16,
        }
    }
}

impl PrivacyConfig {
    pub fn validate(self) -> Result<Self, CoreError> {
        if self.max_generation_attempts == 0 {
            return Err(CoreError::InvalidPrivacyConfig {
                field: "max_generation_attempts",
            });
        }
        Ok(self)
    }
}

impl PeerIdentityManager {
    pub fn new(config: PrivacyConfig) -> Result<Self, CoreError> {
        Ok(Self {
            config: config.validate()?,
            generated: HashSet::new(),
            next_epoch: 0,
        })
    }

    pub fn generate<R>(&mut self, rng: &mut R) -> Result<PeerIdentity, CoreError>
    where
        R: RngCore + CryptoRng,
    {
        for _ in 0..self.config.max_generation_attempts {
            let peer_id = self.candidate_peer_id(rng);
            if self.generated.insert(peer_id) {
                let identity = PeerIdentity {
                    epoch: IdentityEpoch::new(self.next_epoch),
                    peer_id,
                };
                self.next_epoch = self.next_epoch.saturating_add(1);
                return Ok(identity);
            }
        }

        Err(CoreError::PeerIdExhausted {
            attempts: self.config.max_generation_attempts,
        })
    }

    fn candidate_peer_id<R>(&self, rng: &mut R) -> PeerId
    where
        R: RngCore + CryptoRng,
    {
        let mut bytes = [0; 20];
        rng.fill_bytes(&mut bytes);
        if let Some(prefix) = self.config.peer_id_prefix {
            bytes[..8].copy_from_slice(prefix.as_bytes());
        }
        PeerId::new(bytes)
    }
}

impl PrivacyController {
    pub fn new(config: PrivacyConfig) -> Result<Self, CoreError> {
        Ok(Self {
            identities: PeerIdentityManager::new(config)?,
        })
    }

    pub fn apply<R>(
        &mut self,
        intent: PrivacyIntent,
        rng: &mut R,
    ) -> Result<PrivacyAction, CoreError>
    where
        R: RngCore + CryptoRng,
    {
        let identity = self.identities.generate(rng)?;
        match intent {
            PrivacyIntent::NewAnnounceIdentity => Ok(PrivacyAction::UsePeerId { identity }),
            PrivacyIntent::ReconnectWithFreshIdentity { peer }
            | PrivacyIntent::RotatePeerIdentity { peer } => {
                Ok(PrivacyAction::DisconnectForPrivacy { peer, identity })
            }
        }
    }
}
