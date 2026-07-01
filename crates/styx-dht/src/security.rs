use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use std::net::Ipv4Addr;

use crate::NodeId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRateLimiter {
    capacity: usize,
    window: Duration,
    sources: HashMap<IpAddr, SourceWindow>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourceWindow {
    started_at: Instant,
    count: usize,
}

impl SourceRateLimiter {
    #[must_use]
    pub fn new(capacity: usize, window: Duration) -> Self {
        Self {
            capacity,
            window,
            sources: HashMap::new(),
        }
    }

    pub fn check(&mut self, source: IpAddr, now: Instant) -> bool {
        let entry = self.sources.entry(source).or_insert(SourceWindow {
            started_at: now,
            count: 0,
        });
        if now.duration_since(entry.started_at) >= self.window {
            entry.started_at = now;
            entry.count = 0;
        }
        if entry.count >= self.capacity {
            return false;
        }
        entry.count += 1;
        true
    }
}

const IPV4_MASK: [u8; 4] = [0x03, 0x0f, 0x3f, 0xff];
const CRC32C_POLY_REVERSED: u32 = 0x82f6_3b78;

#[must_use]
pub fn generate_bep42_ipv4_id(ip: Ipv4Addr, random: u8, entropy: [u8; 16]) -> NodeId {
    let crc = bep42_crc_ipv4(ip, random);
    let mut id = [0_u8; 20];
    id[0] = (crc >> 24) as u8;
    id[1] = (crc >> 16) as u8;
    id[2] = (((crc >> 8) as u8) & 0xf8) | (random & 0x07);
    id[3..19].copy_from_slice(&entropy);
    id[19] = random;
    NodeId::new(id)
}

#[must_use]
pub fn is_bep42_ipv4_id(ip: Ipv4Addr, node_id: &[u8; 20]) -> bool {
    let random = node_id[19];
    let expected = generate_bep42_ipv4_id(ip, random, [0; 16]);
    node_id[0] == expected.as_bytes()[0]
        && node_id[1] == expected.as_bytes()[1]
        && (node_id[2] & 0xf8) == (expected.as_bytes()[2] & 0xf8)
}

fn bep42_crc_ipv4(ip: Ipv4Addr, random: u8) -> u32 {
    let mut masked = ip.octets();
    for (byte, mask) in masked.iter_mut().zip(IPV4_MASK) {
        *byte &= mask;
    }
    masked[0] |= (random & 0x07) << 5;
    crc32c(&masked)
}

fn crc32c(input: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in input {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            if crc & 1 == 1 {
                crc = (crc >> 1) ^ CRC32C_POLY_REVERSED;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}
