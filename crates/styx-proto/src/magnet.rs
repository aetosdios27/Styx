//! Magnet URI parsing for BitTorrent v1, v2, and hybrid torrents.

use std::net::SocketAddr;

use url::{form_urlencoded, Url};

use crate::{InfoHashV1, InfoHashV2};

const MAGNET_PREFIX: &str = "magnet:?";
const BTIH_PREFIX: &str = "urn:btih:";
const BTMH_PREFIX: &str = "urn:btmh:";
const MULTIHASH_SHA256_PREFIX: &str = "1220";

/// Parsed magnet URI fields used by the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MagnetUri {
    /// v1 SHA-1 info hash from `xt=urn:btih:...`.
    pub info_hash_v1: Option<InfoHashV1>,
    /// v2 SHA-256 info hash from `xt=urn:btmh:1220...`.
    pub info_hash_v2: Option<InfoHashV2>,
    /// Optional display name from `dn`.
    pub display_name: Option<String>,
    /// Tracker URLs from repeated `tr` parameters.
    pub trackers: Vec<Url>,
    /// Exact peers from repeated `x.pe` parameters.
    pub exact_peers: Vec<SocketAddr>,
    /// BEP 53 select-only file indices from `so`.
    pub select_only: Option<Vec<u32>>,
}

/// Errors returned while parsing magnet URIs.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MagnetError {
    /// URI did not use the `magnet:?` form.
    #[error("invalid magnet URI scheme")]
    InvalidScheme,
    /// No supported exact-topic hash was present.
    #[error("magnet URI is missing a supported xt info hash")]
    MissingXt,
    /// Hash text had the wrong byte length.
    #[error("invalid magnet hash length")]
    InvalidHashLength,
    /// Hash text was not valid hex.
    #[error("invalid magnet hash hex")]
    InvalidHex,
    /// v1 base32 info hash contained an invalid character.
    #[error("invalid btih base32 character `{character}`")]
    InvalidBase32 {
        /// Invalid character.
        character: char,
    },
    /// v2 multihash was not SHA-256 using the `1220` prefix.
    #[error("unsupported btmh multihash")]
    UnsupportedMultihash,
    /// A tracker URL could not be parsed.
    #[error("invalid tracker URL `{value}`")]
    InvalidTrackerUrl {
        /// Invalid tracker URL.
        value: String,
    },
    /// An exact peer address could not be parsed.
    #[error("invalid exact peer address `{value}`")]
    InvalidExactPeer {
        /// Invalid peer address.
        value: String,
    },
    /// BEP 53 select-only syntax was invalid.
    #[error("invalid select-only value `{value}`")]
    InvalidSelectOnly {
        /// Invalid select-only value.
        value: String,
    },
}

/// Parse a magnet URI.
///
/// # Errors
///
/// Returns [`MagnetError`] when the URI is not a magnet, contains malformed
/// supported fields, or lacks a supported `xt` info hash.
pub fn parse_magnet_uri(input: &str) -> Result<MagnetUri, MagnetError> {
    let Some(query) = input.strip_prefix(MAGNET_PREFIX) else {
        return Err(MagnetError::InvalidScheme);
    };

    let mut magnet = MagnetUri {
        info_hash_v1: None,
        info_hash_v2: None,
        display_name: None,
        trackers: Vec::new(),
        exact_peers: Vec::new(),
        select_only: None,
    };

    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "xt" => parse_xt(value.as_ref(), &mut magnet)?,
            "dn" => magnet.display_name = Some(value.into_owned()),
            "tr" => magnet.trackers.push(parse_tracker(value.as_ref())?),
            "x.pe" => magnet.exact_peers.push(parse_exact_peer(value.as_ref())?),
            "so" => magnet.select_only = Some(parse_select_only(value.as_ref())?),
            _ => {}
        }
    }

    if magnet.info_hash_v1.is_none() && magnet.info_hash_v2.is_none() {
        return Err(MagnetError::MissingXt);
    }

    Ok(magnet)
}

fn parse_xt(value: &str, magnet: &mut MagnetUri) -> Result<(), MagnetError> {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with(BTIH_PREFIX) {
        magnet.info_hash_v1 = Some(parse_btih(&value[BTIH_PREFIX.len()..])?);
    } else if lower.starts_with(BTMH_PREFIX) {
        magnet.info_hash_v2 = Some(parse_btmh(&value[BTMH_PREFIX.len()..])?);
    }
    Ok(())
}

fn parse_btih(value: &str) -> Result<InfoHashV1, MagnetError> {
    match value.len() {
        40 => parse_btih_hex(value),
        32 => parse_btih_base32(value),
        _ => Err(MagnetError::InvalidHashLength),
    }
}

fn parse_btih_hex(value: &str) -> Result<InfoHashV1, MagnetError> {
    let mut bytes = [0; 20];
    decode_hex_into(value, &mut bytes)?;
    Ok(InfoHashV1::new(bytes))
}

fn parse_btih_base32(value: &str) -> Result<InfoHashV1, MagnetError> {
    let mut output = [0; 20];
    let mut buffer = 0u32;
    let mut bits = 0u8;
    let mut index = 0usize;

    for character in value.chars() {
        let value = base32_value(character)?;
        buffer = (buffer << 5) | u32::from(value);
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            let byte = ((buffer >> bits) & 0xff) as u8;
            let Some(slot) = output.get_mut(index) else {
                return Err(MagnetError::InvalidHashLength);
            };
            *slot = byte;
            index += 1;
        }
    }

    if index != output.len() || bits != 0 {
        return Err(MagnetError::InvalidHashLength);
    }

    Ok(InfoHashV1::new(output))
}

fn parse_btmh(value: &str) -> Result<InfoHashV2, MagnetError> {
    let lower = value.to_ascii_lowercase();
    if !lower.starts_with(MULTIHASH_SHA256_PREFIX) {
        return Err(MagnetError::UnsupportedMultihash);
    }
    let hash = &value[MULTIHASH_SHA256_PREFIX.len()..];
    if hash.len() != 64 {
        return Err(MagnetError::InvalidHashLength);
    }
    let mut bytes = [0; 32];
    decode_hex_into(hash, &mut bytes)?;
    Ok(InfoHashV2::new(bytes))
}

fn decode_hex_into(value: &str, output: &mut [u8]) -> Result<(), MagnetError> {
    if value.len() != output.len() * 2 {
        return Err(MagnetError::InvalidHashLength);
    }
    for (i, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)
            .map_err(|_| MagnetError::InvalidHex)?;
    }
    Ok(())
}

fn base32_value(character: char) -> Result<u8, MagnetError> {
    match character.to_ascii_uppercase() {
        'A'..='Z' => Ok(character.to_ascii_uppercase() as u8 - b'A'),
        '2'..='7' => Ok(character as u8 - b'2' + 26),
        character => Err(MagnetError::InvalidBase32 { character }),
    }
}

fn parse_tracker(value: &str) -> Result<Url, MagnetError> {
    Url::parse(value).map_err(|_| MagnetError::InvalidTrackerUrl {
        value: value.to_string(),
    })
}

fn parse_exact_peer(value: &str) -> Result<SocketAddr, MagnetError> {
    let peer = value
        .parse::<SocketAddr>()
        .map_err(|_| MagnetError::InvalidExactPeer {
            value: value.to_string(),
        })?;
    if peer.port() == 0 {
        return Err(MagnetError::InvalidExactPeer {
            value: value.to_string(),
        });
    }
    Ok(peer)
}

fn parse_select_only(value: &str) -> Result<Vec<u32>, MagnetError> {
    if value.is_empty() {
        return Err(MagnetError::InvalidSelectOnly {
            value: value.to_string(),
        });
    }

    let mut indices = Vec::new();
    for part in value.split(',') {
        if let Some((start, end)) = part.split_once('-') {
            let start = parse_select_index(value, start)?;
            let end = parse_select_index(value, end)?;
            if start > end {
                return Err(MagnetError::InvalidSelectOnly {
                    value: value.to_string(),
                });
            }
            indices.extend(start..=end);
        } else {
            indices.push(parse_select_index(value, part)?);
        }
    }
    indices.sort_unstable();
    indices.dedup();

    Ok(indices)
}

fn parse_select_index(full_value: &str, value: &str) -> Result<u32, MagnetError> {
    if value.is_empty() {
        return Err(MagnetError::InvalidSelectOnly {
            value: full_value.to_string(),
        });
    }
    value.parse().map_err(|_| MagnetError::InvalidSelectOnly {
        value: full_value.to_string(),
    })
}
