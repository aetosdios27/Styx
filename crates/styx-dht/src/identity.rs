use std::collections::HashSet;

use rand::{CryptoRng, RngCore};

use crate::{
    generate_bep42_ipv4_id, generate_bep42_ipv6_id, is_bep42_id, DhtError, ExternalIp, NodeId,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DhtIdentityEpoch(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhtIdentity {
    pub epoch: DhtIdentityEpoch,
    pub node_id: NodeId,
    pub external_ip: Option<ExternalIp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DhtIdentityAction {
    RestartWithNodeId { identity: DhtIdentity },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DhtIdentityManager {
    current: Option<DhtIdentity>,
    generated: HashSet<NodeId>,
    next_epoch: u64,
    max_generation_attempts: usize,
}

impl Default for DhtIdentityManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DhtIdentityEpoch {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl DhtIdentityManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            current: None,
            generated: HashSet::new(),
            next_epoch: 0,
            max_generation_attempts: 16,
        }
    }

    pub fn with_max_generation_attempts(max_generation_attempts: usize) -> Result<Self, DhtError> {
        if max_generation_attempts == 0 {
            return Err(DhtError::InvalidIdentityConfig("max_generation_attempts"));
        }
        Ok(Self {
            max_generation_attempts,
            ..Self::new()
        })
    }

    pub fn rotate_without_external_ip<R>(&mut self, rng: &mut R) -> Result<DhtIdentity, DhtError>
    where
        R: RngCore + CryptoRng,
    {
        self.rotate(None, rng)
    }

    pub fn observe_external_ip<R>(
        &mut self,
        external_ip: ExternalIp,
        rng: &mut R,
    ) -> Result<Option<DhtIdentityAction>, DhtError>
    where
        R: RngCore + CryptoRng,
    {
        if self.current.is_some_and(|identity| {
            identity.external_ip == Some(external_ip)
                && is_bep42_id(external_ip, identity.node_id.as_bytes())
        }) {
            return Ok(None);
        }

        let identity = self.rotate(Some(external_ip), rng)?;
        Ok(Some(DhtIdentityAction::RestartWithNodeId { identity }))
    }

    #[must_use]
    pub const fn current(&self) -> Option<DhtIdentity> {
        self.current
    }

    fn rotate<R>(
        &mut self,
        external_ip: Option<ExternalIp>,
        rng: &mut R,
    ) -> Result<DhtIdentity, DhtError>
    where
        R: RngCore + CryptoRng,
    {
        for _ in 0..self.max_generation_attempts {
            let node_id = generate_node_id(external_ip, rng);
            if self.generated.insert(node_id) {
                let identity = DhtIdentity {
                    epoch: DhtIdentityEpoch::new(self.next_epoch),
                    node_id,
                    external_ip,
                };
                self.next_epoch = self.next_epoch.saturating_add(1);
                self.current = Some(identity);
                return Ok(identity);
            }
        }

        Err(DhtError::NodeIdExhausted {
            attempts: self.max_generation_attempts,
        })
    }
}

fn generate_node_id<R>(external_ip: Option<ExternalIp>, rng: &mut R) -> NodeId
where
    R: RngCore + CryptoRng,
{
    match external_ip {
        Some(ExternalIp::V4(ip)) => {
            let (random, entropy) = random_bep42_parts(rng);
            generate_bep42_ipv4_id(ip, random, entropy)
        }
        Some(ExternalIp::V6(ip)) => {
            let (random, entropy) = random_bep42_parts(rng);
            generate_bep42_ipv6_id(ip, random, entropy)
        }
        None => {
            let mut bytes = [0; 20];
            rng.fill_bytes(&mut bytes);
            NodeId::new(bytes)
        }
    }
}

fn random_bep42_parts<R>(rng: &mut R) -> (u8, [u8; 16])
where
    R: RngCore + CryptoRng,
{
    let mut entropy = [0; 16];
    rng.fill_bytes(&mut entropy);
    let random = (rng.next_u32() & 0xff) as u8;
    (random, entropy)
}
