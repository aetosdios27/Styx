use crate::info_hash_v2::InfoHashV2;

const BTMH_PREFIX: &str = "urn:btmh:";
const BTIH_PREFIX: &str = "urn:btih:";

/// The multihash prefix for SHA-256: 0x12 (hash function) 0x20 (32 bytes)
const MULTIHASH_SHA256_PREFIX: &str = "1220";

#[derive(Debug, Clone)]
pub struct V2MagnetInfo {
    pub info_hash_v2: Option<InfoHashV2>,
    pub info_hash: Option<[u8; 20]>,
    pub display_name: Option<String>,
    pub trackers: Vec<String>,
}

/// Parse a magnet link that may contain v2 (btmh) and/or v1 (btih) info hashes.
pub fn parse_v2_magnet(uri: &str) -> Result<V2MagnetInfo, MagnetV2Error> {
    if !uri.starts_with("magnet:?") {
        return Err(MagnetV2Error::InvalidScheme);
    }

    let query = &uri[8..];
    let mut info_hash_v2 = None;
    let mut info_hash = None;
    let mut display_name = None;
    let mut trackers = Vec::new();

    for param in query.split('&') {
        if let Some((key, value)) = param.split_once('=') {
            match key {
                "xt" => {
                    if let Some(btmh_hash) = value.strip_prefix(BTMH_PREFIX) {
                        info_hash_v2 = Some(parse_btmh_hash(btmh_hash)?);
                    } else if let Some(btih_hash) = value.strip_prefix(BTIH_PREFIX) {
                        info_hash = Some(parse_btih_hash(btih_hash)?);
                    }
                }
                "dn" => {
                    display_name = Some(url_decode(value));
                }
                "tr" => {
                    trackers.push(url_decode(value));
                }
                _ => {}
            }
        }
    }

    Ok(V2MagnetInfo {
        info_hash_v2,
        info_hash,
        display_name,
        trackers,
    })
}

fn parse_btmh_hash(hex: &str) -> Result<InfoHashV2, MagnetV2Error> {
    if !hex.starts_with(MULTIHASH_SHA256_PREFIX) {
        return Err(MagnetV2Error::UnsupportedMultihash);
    }

    let hash_hex = &hex[MULTIHASH_SHA256_PREFIX.len()..];
    if hash_hex.len() != 64 {
        return Err(MagnetV2Error::InvalidHashLength);
    }

    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hash_hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| MagnetV2Error::InvalidHex)?;
    }

    Ok(InfoHashV2::from(&bytes))
}

fn parse_btih_hash(hex: &str) -> Result<[u8; 20], MagnetV2Error> {
    if hex.len() != 40 {
        return Err(MagnetV2Error::InvalidHashLength);
    }
    let mut bytes = [0u8; 20];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| MagnetV2Error::InvalidHex)?;
    }
    Ok(bytes)
}

fn url_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MagnetV2Error {
    #[error("invalid magnet URI scheme")]
    InvalidScheme,
    #[error("unsupported multihash format (expected 1220 for SHA-256)")]
    UnsupportedMultihash,
    #[error("invalid hash length")]
    InvalidHashLength,
    #[error("invalid hex encoding")]
    InvalidHex,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_btmh_magnet_link() {
        let magnet = "magnet:?xt=urn:btmh:1220caf1e1c30e81cb361b9ee167c4aa64228a7fa4fa9f6105232b28ad099f3a302e&dn=test";
        let parsed = parse_v2_magnet(magnet).unwrap();
        assert!(parsed.info_hash_v2.is_some());
        assert_eq!(parsed.info_hash_v2.unwrap().as_bytes().len(), 32);
        assert_eq!(parsed.display_name, Some("test".to_string()));
    }

    #[test]
    fn parse_dual_magnet_link() {
        let magnet = "magnet:?xt=urn:btih:631a31dd0a46257d5078c0dee4e66e26f73e42ac&xt=urn:btmh:1220d8dd32ac93357c368556af3ac1d95c9d76bd0dff6fa9833ecdac3d53134efabb&dn=test";
        let parsed = parse_v2_magnet(magnet).unwrap();
        assert!(parsed.info_hash_v2.is_some());
        let expected: [u8; 20] = [
            0x63, 0x1a, 0x31, 0xdd, 0x0a, 0x46, 0x25, 0x7d, 0x50, 0x78,
            0xc0, 0xde, 0xe4, 0xe6, 0x6e, 0x26, 0xf7, 0x3e, 0x42, 0xac,
        ];
        assert_eq!(parsed.info_hash.unwrap(), expected);
    }
}
