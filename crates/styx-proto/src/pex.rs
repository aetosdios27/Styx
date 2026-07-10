//! Bounded BEP 11 peer-exchange message codec.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use bytes::{BufMut, Bytes, BytesMut};

use crate::{decode, encode, BencodeError, BencodeValue};

/// Maximum contacts accepted per address family and operation.
pub const MAX_PEX_CONTACTS_PER_FAMILY: usize = 50;

/// A BEP 11 `ut_pex` payload.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PexMessage {
    pub added: Vec<SocketAddr>,
    pub added6: Vec<SocketAddr>,
    pub dropped: Vec<SocketAddr>,
    pub dropped6: Vec<SocketAddr>,
    pub added_flags: Vec<u8>,
    pub added6_flags: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PexError {
    #[error(transparent)]
    Bencode(#[from] BencodeError),
    #[error("PEX payload must be a dictionary")]
    ExpectedDictionary,
    #[error("PEX field `{field}` must be a byte string")]
    InvalidFieldType { field: String },
    #[error("PEX field `{field}` length must be a multiple of {width}")]
    InvalidCompactLength { field: String, width: usize },
    #[error("PEX field `{field}` contains {actual} contacts; maximum is 50")]
    TooManyContacts { field: String, actual: usize },
    #[error("PEX field `{field}` flags length must equal its contact count")]
    InvalidFlagsLength { field: String },
    #[error("PEX field `{field}` contains an address from the wrong family")]
    WrongAddressFamily { field: String },
}

pub fn decode_pex_message(input: &[u8]) -> Result<PexMessage, PexError> {
    let BencodeValue::Dict(fields) = decode(input)? else {
        return Err(PexError::ExpectedDictionary);
    };
    let mut message = PexMessage::default();
    for (key, value) in fields {
        match key.as_slice() {
            b"added" => message.added = decode_v4("added", bytes("added", value)?)?,
            b"added6" => message.added6 = decode_v6("added6", bytes("added6", value)?)?,
            b"dropped" => message.dropped = decode_v4("dropped", bytes("dropped", value)?)?,
            b"dropped6" => {
                message.dropped6 = decode_v6("dropped6", bytes("dropped6", value)?)?;
            }
            b"added.f" => message.added_flags = bytes("added.f", value)?.to_vec(),
            b"added6.f" => message.added6_flags = bytes("added6.f", value)?.to_vec(),
            _ => {}
        }
    }
    validate_flags("added.f", message.added.len(), &message.added_flags)?;
    validate_flags("added6.f", message.added6.len(), &message.added6_flags)?;
    Ok(message)
}

pub fn encode_pex_message(message: &PexMessage) -> Result<Bytes, PexError> {
    validate_contacts("added", &message.added)?;
    validate_contacts("added6", &message.added6)?;
    validate_contacts("dropped", &message.dropped)?;
    validate_contacts("dropped6", &message.dropped6)?;
    validate_flags("added.f", message.added.len(), &message.added_flags)?;
    validate_flags("added6.f", message.added6.len(), &message.added6_flags)?;

    let mut fields = BTreeMap::new();
    fields.insert(
        b"added".to_vec(),
        BencodeValue::Bytes(encode_v4("added", &message.added)?),
    );
    fields.insert(
        b"added6".to_vec(),
        BencodeValue::Bytes(encode_v6("added6", &message.added6)?),
    );
    fields.insert(
        b"dropped".to_vec(),
        BencodeValue::Bytes(encode_v4("dropped", &message.dropped)?),
    );
    fields.insert(
        b"dropped6".to_vec(),
        BencodeValue::Bytes(encode_v6("dropped6", &message.dropped6)?),
    );
    if !message.added_flags.is_empty() {
        fields.insert(
            b"added.f".to_vec(),
            BencodeValue::Bytes(Bytes::copy_from_slice(&message.added_flags)),
        );
    }
    if !message.added6_flags.is_empty() {
        fields.insert(
            b"added6.f".to_vec(),
            BencodeValue::Bytes(Bytes::copy_from_slice(&message.added6_flags)),
        );
    }
    Ok(Bytes::from(encode(&BencodeValue::Dict(fields))))
}

fn bytes(field: &str, value: BencodeValue) -> Result<Bytes, PexError> {
    match value {
        BencodeValue::Bytes(value) => Ok(value),
        _ => Err(PexError::InvalidFieldType {
            field: field.to_owned(),
        }),
    }
}

fn validate_contacts(field: &str, contacts: &[SocketAddr]) -> Result<(), PexError> {
    if contacts.len() > MAX_PEX_CONTACTS_PER_FAMILY {
        return Err(PexError::TooManyContacts {
            field: field.to_owned(),
            actual: contacts.len(),
        });
    }
    Ok(())
}

fn validate_flags(field: &str, contacts: usize, flags: &[u8]) -> Result<(), PexError> {
    if !flags.is_empty() && flags.len() != contacts {
        return Err(PexError::InvalidFlagsLength {
            field: field.to_owned(),
        });
    }
    Ok(())
}

fn decode_v4(field: &str, bytes: Bytes) -> Result<Vec<SocketAddr>, PexError> {
    decode_compact(field, bytes, 6, |chunk| {
        SocketAddr::from((
            Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]),
            u16::from_be_bytes([chunk[4], chunk[5]]),
        ))
    })
}

fn decode_v6(field: &str, bytes: Bytes) -> Result<Vec<SocketAddr>, PexError> {
    decode_compact(field, bytes, 18, |chunk| {
        let mut ip = [0; 16];
        ip.copy_from_slice(&chunk[..16]);
        SocketAddr::from((
            Ipv6Addr::from(ip),
            u16::from_be_bytes([chunk[16], chunk[17]]),
        ))
    })
}

fn decode_compact(
    field: &str,
    bytes: Bytes,
    width: usize,
    decode: impl Fn(&[u8]) -> SocketAddr,
) -> Result<Vec<SocketAddr>, PexError> {
    if !bytes.len().is_multiple_of(width) {
        return Err(PexError::InvalidCompactLength {
            field: field.to_owned(),
            width,
        });
    }
    let count = bytes.len() / width;
    if count > MAX_PEX_CONTACTS_PER_FAMILY {
        return Err(PexError::TooManyContacts {
            field: field.to_owned(),
            actual: count,
        });
    }
    Ok(bytes.chunks_exact(width).map(decode).collect())
}

fn encode_v4(field: &str, contacts: &[SocketAddr]) -> Result<Bytes, PexError> {
    let mut bytes = BytesMut::with_capacity(contacts.len() * 6);
    for contact in contacts {
        let SocketAddr::V4(contact) = contact else {
            return Err(PexError::WrongAddressFamily {
                field: field.to_owned(),
            });
        };
        bytes.extend_from_slice(&contact.ip().octets());
        bytes.put_u16(contact.port());
    }
    Ok(bytes.freeze())
}

fn encode_v6(field: &str, contacts: &[SocketAddr]) -> Result<Bytes, PexError> {
    let mut bytes = BytesMut::with_capacity(contacts.len() * 18);
    for contact in contacts {
        let SocketAddr::V6(contact) = contact else {
            return Err(PexError::WrongAddressFamily {
                field: field.to_owned(),
            });
        };
        bytes.extend_from_slice(&contact.ip().octets());
        bytes.put_u16(contact.port());
    }
    Ok(bytes.freeze())
}
