//! BEP 10 extension protocol handshake parsing.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};

use bytes::Bytes;

use crate::bencode::{decode, encode, BencodeError, BencodeValue};

/// BEP 10 extension handshake payload.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtensionHandshake {
    /// Peer-local extension message ids keyed by extension name.
    pub messages: BTreeMap<String, u8>,
    /// Optional BEP 9 metadata byte length.
    pub metadata_size: Option<u64>,
    /// Optional peer TCP listen port.
    pub listen_port: Option<u16>,
    /// Optional client/version string supplied by the peer.
    pub client: Option<String>,
    /// Optional IPv4 address observed through `yourip`.
    pub ipv4: Option<Ipv4Addr>,
    /// Optional IPv6 address observed through `ipv6` or 16-byte `yourip`.
    pub ipv6: Option<Ipv6Addr>,
}

impl ExtensionHandshake {
    /// Return the peer-local id for `name`, treating id `0` as disabled.
    #[must_use]
    pub fn message_id(&self, name: &str) -> Option<u8> {
        self.messages
            .get(name)
            .copied()
            .and_then(|id| (id != 0).then_some(id))
    }
}

/// Errors returned while parsing BEP 10 extension handshakes.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ExtensionError {
    /// The payload was not valid bencode.
    #[error(transparent)]
    Bencode(#[from] BencodeError),
    /// The top-level payload was not a bencoded dictionary.
    #[error("extension handshake payload must be a dictionary")]
    ExpectedDictionary,
    /// A known field had the wrong bencode type.
    #[error("extension handshake field `{field}` has invalid type")]
    InvalidFieldType {
        /// Field name.
        field: &'static str,
    },
    /// A known byte-string field had an invalid byte length.
    #[error(
        "extension handshake field `{field}` has invalid length {actual}, expected {expected}"
    )]
    InvalidFieldLength {
        /// Field name.
        field: &'static str,
        /// Actual byte length.
        actual: usize,
        /// Expected byte length description.
        expected: &'static str,
    },
    /// An extension message id was not in the valid `0..=255` range.
    #[error("extension message id for `{name}` is out of range: {id}")]
    InvalidMessageId {
        /// Extension name.
        name: String,
        /// Invalid id.
        id: i64,
    },
    /// A known string field was not valid UTF-8.
    #[error("extension handshake field `{field}` is not valid UTF-8")]
    InvalidUtf8 {
        /// Field name.
        field: &'static str,
    },
    /// An integer field was negative or too large for its target type.
    #[error("extension handshake field `{field}` has invalid integer value {value}")]
    InvalidIntegerValue {
        /// Field name.
        field: &'static str,
        /// Invalid value.
        value: i64,
    },
}

/// Decode a BEP 10 extension handshake payload.
///
/// # Errors
///
/// Returns [`ExtensionError`] when the payload is not a dictionary or a known
/// field has the wrong type, length, UTF-8 encoding, or integer range.
pub fn decode_extension_handshake(input: &[u8]) -> Result<ExtensionHandshake, ExtensionError> {
    let BencodeValue::Dict(fields) = decode(input)? else {
        return Err(ExtensionError::ExpectedDictionary);
    };

    let mut handshake = ExtensionHandshake::default();
    for (key, value) in fields {
        match key.as_slice() {
            b"m" => handshake.messages = parse_messages(value)?,
            b"metadata_size" => {
                handshake.metadata_size = Some(parse_nonnegative_u64("metadata_size", value)?);
            }
            b"p" => handshake.listen_port = Some(parse_u16("p", value)?),
            b"v" => handshake.client = Some(parse_utf8_bytes("v", value)?),
            b"yourip" => parse_yourip(value, &mut handshake)?,
            b"ipv6" => handshake.ipv6 = Some(parse_ipv6("ipv6", value)?),
            _ => {}
        }
    }

    Ok(handshake)
}

/// Encode a BEP 10 extension handshake payload.
#[must_use]
pub fn encode_extension_handshake(handshake: &ExtensionHandshake) -> Vec<u8> {
    let mut fields = BTreeMap::new();
    fields.insert(b"m".to_vec(), encode_messages(&handshake.messages));

    if let Some(metadata_size) = handshake.metadata_size {
        fields.insert(
            b"metadata_size".to_vec(),
            BencodeValue::Integer(metadata_size as i64),
        );
    }
    if let Some(port) = handshake.listen_port {
        fields.insert(b"p".to_vec(), BencodeValue::Integer(i64::from(port)));
    }
    if let Some(client) = handshake.client.as_ref() {
        fields.insert(
            b"v".to_vec(),
            BencodeValue::Bytes(Bytes::copy_from_slice(client.as_bytes())),
        );
    }
    if let Some(ipv4) = handshake.ipv4 {
        fields.insert(
            b"yourip".to_vec(),
            BencodeValue::Bytes(Bytes::copy_from_slice(&ipv4.octets())),
        );
    }
    if let Some(ipv6) = handshake.ipv6 {
        fields.insert(
            b"ipv6".to_vec(),
            BencodeValue::Bytes(Bytes::copy_from_slice(&ipv6.octets())),
        );
    }

    encode(&BencodeValue::Dict(fields))
}

fn parse_messages(value: BencodeValue) -> Result<BTreeMap<String, u8>, ExtensionError> {
    let BencodeValue::Dict(messages) = value else {
        return Err(ExtensionError::InvalidFieldType { field: "m" });
    };

    let mut parsed = BTreeMap::new();
    for (name, id) in messages {
        let name =
            String::from_utf8(name).map_err(|_| ExtensionError::InvalidUtf8 { field: "m" })?;
        let BencodeValue::Integer(id) = id else {
            return Err(ExtensionError::InvalidFieldType { field: "m" });
        };
        let id = u8::try_from(id).map_err(|_| ExtensionError::InvalidMessageId {
            name: name.clone(),
            id,
        })?;
        parsed.insert(name, id);
    }
    Ok(parsed)
}

fn encode_messages(messages: &BTreeMap<String, u8>) -> BencodeValue {
    let values = messages
        .iter()
        .map(|(name, id)| {
            (
                name.as_bytes().to_vec(),
                BencodeValue::Integer(i64::from(*id)),
            )
        })
        .collect();
    BencodeValue::Dict(values)
}

fn parse_nonnegative_u64(field: &'static str, value: BencodeValue) -> Result<u64, ExtensionError> {
    let BencodeValue::Integer(value) = value else {
        return Err(ExtensionError::InvalidFieldType { field });
    };
    u64::try_from(value).map_err(|_| ExtensionError::InvalidIntegerValue { field, value })
}

fn parse_u16(field: &'static str, value: BencodeValue) -> Result<u16, ExtensionError> {
    let BencodeValue::Integer(value) = value else {
        return Err(ExtensionError::InvalidFieldType { field });
    };
    u16::try_from(value).map_err(|_| ExtensionError::InvalidIntegerValue { field, value })
}

fn parse_utf8_bytes(field: &'static str, value: BencodeValue) -> Result<String, ExtensionError> {
    let BencodeValue::Bytes(bytes) = value else {
        return Err(ExtensionError::InvalidFieldType { field });
    };
    String::from_utf8(bytes.to_vec()).map_err(|_| ExtensionError::InvalidUtf8 { field })
}

fn parse_yourip(
    value: BencodeValue,
    handshake: &mut ExtensionHandshake,
) -> Result<(), ExtensionError> {
    let BencodeValue::Bytes(bytes) = value else {
        return Err(ExtensionError::InvalidFieldType { field: "yourip" });
    };
    match bytes.len() {
        4 => {
            handshake.ipv4 = Some(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]));
            Ok(())
        }
        16 => {
            let mut octets = [0; 16];
            octets.copy_from_slice(&bytes);
            handshake.ipv6 = Some(Ipv6Addr::from(octets));
            Ok(())
        }
        actual => Err(ExtensionError::InvalidFieldLength {
            field: "yourip",
            actual,
            expected: "4 or 16",
        }),
    }
}

fn parse_ipv6(field: &'static str, value: BencodeValue) -> Result<Ipv6Addr, ExtensionError> {
    let BencodeValue::Bytes(bytes) = value else {
        return Err(ExtensionError::InvalidFieldType { field });
    };
    if bytes.len() != 16 {
        return Err(ExtensionError::InvalidFieldLength {
            field,
            actual: bytes.len(),
            expected: "16",
        });
    }
    let mut octets = [0; 16];
    octets.copy_from_slice(&bytes);
    Ok(Ipv6Addr::from(octets))
}
