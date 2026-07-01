use std::net::IpAddr;

use bytes::Bytes;
use sha1::{Digest, Sha1};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenManager {
    current_secret: Bytes,
    previous_secret: Bytes,
}

impl TokenManager {
    #[must_use]
    pub fn with_secrets(current_secret: Bytes, previous_secret: Bytes) -> Self {
        Self {
            current_secret,
            previous_secret,
        }
    }

    pub fn rotate(&mut self, next_secret: Bytes) {
        self.previous_secret = self.current_secret.clone();
        self.current_secret = next_secret;
    }

    #[must_use]
    pub fn issue(&self, ip: IpAddr) -> Bytes {
        token_for(ip, &self.current_secret)
    }

    #[must_use]
    pub fn validate(&self, ip: IpAddr, token: &[u8]) -> bool {
        token == token_for(ip, &self.current_secret).as_ref()
            || token == token_for(ip, &self.previous_secret).as_ref()
    }
}

fn token_for(ip: IpAddr, secret: &[u8]) -> Bytes {
    let mut hasher = Sha1::new();
    match ip {
        IpAddr::V4(ip) => hasher.update(ip.octets()),
        IpAddr::V6(ip) => hasher.update(ip.octets()),
    }
    hasher.update(secret);
    Bytes::copy_from_slice(&hasher.finalize()[..8])
}
