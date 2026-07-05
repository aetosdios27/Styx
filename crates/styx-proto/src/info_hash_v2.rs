//! SHA-256 info hash type for BitTorrent v2.

use core::fmt;

pub const SHA256_DIGEST_BYTES: usize = 32;

/// A BitTorrent v2 info hash (SHA-256 digest).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct InfoHashV2([u8; SHA256_DIGEST_BYTES]);

impl InfoHashV2 {
    /// Construct a v2 info hash from its raw 32-byte SHA-256 digest.
    #[must_use]
    pub const fn new(bytes: [u8; SHA256_DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    /// Return the raw 32-byte SHA-256 digest.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; SHA256_DIGEST_BYTES] {
        &self.0
    }

    /// Truncate to 20 bytes for DHT and tracker announces (BEP 52 §trackers).
    #[must_use]
    pub fn truncated_for_dht(&self) -> [u8; 20] {
        let mut truncated = [0u8; 20];
        truncated.copy_from_slice(&self.0[..20]);
        truncated
    }
}

impl From<[u8; SHA256_DIGEST_BYTES]> for InfoHashV2 {
    fn from(bytes: [u8; SHA256_DIGEST_BYTES]) -> Self {
        Self(bytes)
    }
}

impl From<&[u8; SHA256_DIGEST_BYTES]> for InfoHashV2 {
    fn from(bytes: &[u8; SHA256_DIGEST_BYTES]) -> Self {
        Self(*bytes)
    }
}

impl fmt::Display for InfoHashV2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
