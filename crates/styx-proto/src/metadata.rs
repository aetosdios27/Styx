//! BEP 9 metadata exchange message parsing.

use std::collections::BTreeMap;

use bytes::Bytes;

use crate::bencode::{decode, encode, BencodeError, BencodeValue};

/// BEP 9 metadata block size in bytes.
pub const METADATA_BLOCK_LEN: u64 = 16 * 1024;

/// BEP 9 metadata extension message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetadataMessage {
    /// Request one metadata piece.
    Request {
        /// Zero-based metadata piece index.
        piece: u32,
    },
    /// Metadata piece payload.
    Data {
        /// Zero-based metadata piece index.
        piece: u32,
        /// Total metadata byte size.
        total_size: u64,
        /// Raw metadata block payload appended after the bencoded header.
        payload: Bytes,
    },
    /// Reject one metadata piece request.
    Reject {
        /// Zero-based metadata piece index.
        piece: u32,
    },
}

/// Errors returned while parsing BEP 9 metadata messages.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MetadataError {
    /// The bencoded message header was malformed.
    #[error(transparent)]
    Bencode(#[from] BencodeError),
    /// The message header was not a dictionary.
    #[error("metadata message header must be a dictionary")]
    ExpectedDictionary,
    /// A required field was not present.
    #[error("metadata message missing required field `{field}`")]
    MissingField {
        /// Field name.
        field: &'static str,
    },
    /// A known field had the wrong bencode type.
    #[error("metadata message field `{field}` has invalid type")]
    InvalidFieldType {
        /// Field name.
        field: &'static str,
    },
    /// `msg_type` was not defined by BEP 9.
    #[error("unknown metadata message type {msg_type}")]
    UnknownMessageType {
        /// Unknown message type.
        msg_type: i64,
    },
    /// The metadata piece index was negative or too large.
    #[error("invalid metadata piece index {piece}")]
    InvalidPiece {
        /// Invalid piece index.
        piece: i64,
    },
    /// Runtime metadata total size was zero.
    #[error("invalid metadata total size {total_size}")]
    InvalidTotalSize {
        /// Invalid total size.
        total_size: u64,
    },
    /// Parsed metadata total size was negative or too large.
    #[error("invalid metadata total size value {total_size}")]
    InvalidTotalSizeValue {
        /// Invalid total size value.
        total_size: i64,
    },
    /// A data message did not append metadata bytes after the header.
    #[error("metadata data message is missing payload")]
    MissingPayload,
    /// A request or reject message had trailing bytes after its header.
    #[error("metadata message has unexpected payload bytes: {bytes}")]
    UnexpectedPayload {
        /// Number of unexpected bytes.
        bytes: usize,
    },
    /// The metadata piece count does not fit in `u32`.
    #[error("metadata piece count {count} exceeds u32")]
    PieceCountOverflow {
        /// Overflowing count.
        count: u64,
    },
}

/// Encode a BEP 9 metadata message.
#[must_use]
pub fn encode_metadata_message(message: &MetadataMessage) -> Vec<u8> {
    let mut fields = BTreeMap::new();
    match message {
        MetadataMessage::Request { piece } => {
            fields.insert(b"msg_type".to_vec(), BencodeValue::Integer(0));
            fields.insert(b"piece".to_vec(), BencodeValue::Integer(i64::from(*piece)));
            encode(&BencodeValue::Dict(fields))
        }
        MetadataMessage::Data {
            piece,
            total_size,
            payload,
        } => {
            fields.insert(b"msg_type".to_vec(), BencodeValue::Integer(1));
            fields.insert(b"piece".to_vec(), BencodeValue::Integer(i64::from(*piece)));
            fields.insert(
                b"total_size".to_vec(),
                BencodeValue::Integer(*total_size as i64),
            );
            let mut encoded = encode(&BencodeValue::Dict(fields));
            encoded.extend_from_slice(payload);
            encoded
        }
        MetadataMessage::Reject { piece } => {
            fields.insert(b"msg_type".to_vec(), BencodeValue::Integer(2));
            fields.insert(b"piece".to_vec(), BencodeValue::Integer(i64::from(*piece)));
            encode(&BencodeValue::Dict(fields))
        }
    }
}

/// Decode a BEP 9 metadata message.
///
/// # Errors
///
/// Returns [`MetadataError`] when the bencoded header is malformed, required
/// fields are absent, field values are out of range, or payload bytes violate
/// the message kind.
pub fn decode_metadata_message(input: &[u8]) -> Result<MetadataMessage, MetadataError> {
    let header_end = bencode_prefix_end(input)?;
    let header = decode(&input[..header_end])?;
    let payload = &input[header_end..];
    let BencodeValue::Dict(fields) = header else {
        return Err(MetadataError::ExpectedDictionary);
    };

    let msg_type = required_integer(&fields, b"msg_type", "msg_type")?;
    let piece = parse_piece(required_integer(&fields, b"piece", "piece")?)?;

    match msg_type {
        0 => {
            reject_payload(payload)?;
            Ok(MetadataMessage::Request { piece })
        }
        1 => {
            if payload.is_empty() {
                return Err(MetadataError::MissingPayload);
            }
            let total_size =
                parse_total_size(required_integer(&fields, b"total_size", "total_size")?)?;
            Ok(MetadataMessage::Data {
                piece,
                total_size,
                payload: Bytes::copy_from_slice(payload),
            })
        }
        2 => {
            reject_payload(payload)?;
            Ok(MetadataMessage::Reject { piece })
        }
        msg_type => Err(MetadataError::UnknownMessageType { msg_type }),
    }
}

/// Return how many 16 KiB BEP 9 pieces are required for `total_size`.
///
/// # Errors
///
/// Returns [`MetadataError::InvalidTotalSize`] for zero bytes and
/// [`MetadataError::PieceCountOverflow`] if the count cannot fit in `u32`.
pub fn metadata_piece_count(total_size: u64) -> Result<u32, MetadataError> {
    if total_size == 0 {
        return Err(MetadataError::InvalidTotalSize { total_size });
    }
    let count = total_size.div_ceil(METADATA_BLOCK_LEN);
    u32::try_from(count).map_err(|_| MetadataError::PieceCountOverflow { count })
}

fn reject_payload(payload: &[u8]) -> Result<(), MetadataError> {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(MetadataError::UnexpectedPayload {
            bytes: payload.len(),
        })
    }
}

fn required_integer(
    fields: &BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
    field: &'static str,
) -> Result<i64, MetadataError> {
    let Some(value) = fields.get(key) else {
        return Err(MetadataError::MissingField { field });
    };
    let BencodeValue::Integer(value) = value else {
        return Err(MetadataError::InvalidFieldType { field });
    };
    Ok(*value)
}

fn parse_piece(piece: i64) -> Result<u32, MetadataError> {
    u32::try_from(piece).map_err(|_| MetadataError::InvalidPiece { piece })
}

fn parse_total_size(total_size: i64) -> Result<u64, MetadataError> {
    u64::try_from(total_size)
        .map_err(|_| MetadataError::InvalidTotalSizeValue { total_size })
        .and_then(metadata_total_size_nonzero)
}

fn metadata_total_size_nonzero(total_size: u64) -> Result<u64, MetadataError> {
    if total_size == 0 {
        Err(MetadataError::InvalidTotalSize { total_size })
    } else {
        Ok(total_size)
    }
}

fn bencode_prefix_end(input: &[u8]) -> Result<usize, MetadataError> {
    parse_value_end(input, 0, 0).map_err(MetadataError::Bencode)
}

fn parse_value_end(input: &[u8], offset: usize, depth: usize) -> Result<usize, BencodeError> {
    if depth > 128 {
        return Err(BencodeError::DepthLimitExceeded { offset, limit: 128 });
    }
    let Some(&byte) = input.get(offset) else {
        return Err(BencodeError::UnexpectedEof { offset });
    };
    match byte {
        b'i' => parse_integer_end(input, offset),
        b'l' => parse_list_end(input, offset, depth),
        b'd' => parse_dict_end(input, offset, depth),
        b'0'..=b'9' => parse_bytes_end(input, offset),
        byte => Err(BencodeError::InvalidToken { offset, byte }),
    }
}

fn parse_integer_end(input: &[u8], offset: usize) -> Result<usize, BencodeError> {
    let mut cursor = offset + 1;
    while matches!(input.get(cursor), Some(b'0'..=b'9' | b'-')) {
        cursor += 1;
    }
    if input.get(cursor) == Some(&b'e') {
        Ok(cursor + 1)
    } else if cursor >= input.len() {
        Err(BencodeError::UnexpectedEof { offset: cursor })
    } else {
        Err(BencodeError::InvalidInteger { offset })
    }
}

fn parse_list_end(input: &[u8], offset: usize, depth: usize) -> Result<usize, BencodeError> {
    let mut cursor = offset + 1;
    loop {
        match input.get(cursor) {
            Some(b'e') => return Ok(cursor + 1),
            Some(_) => cursor = parse_value_end(input, cursor, depth + 1)?,
            None => return Err(BencodeError::UnexpectedEof { offset: cursor }),
        }
    }
}

fn parse_dict_end(input: &[u8], offset: usize, depth: usize) -> Result<usize, BencodeError> {
    parse_list_end(input, offset, depth)
}

fn parse_bytes_end(input: &[u8], offset: usize) -> Result<usize, BencodeError> {
    let mut cursor = offset;
    while matches!(input.get(cursor), Some(b'0'..=b'9')) {
        cursor += 1;
    }
    if input.get(cursor) != Some(&b':') {
        return if cursor >= input.len() {
            Err(BencodeError::UnexpectedEof { offset: cursor })
        } else {
            Err(BencodeError::InvalidByteStringLength { offset })
        };
    }
    let digits = &input[offset..cursor];
    if digits.is_empty() || (digits.len() > 1 && digits[0] == b'0') {
        return Err(BencodeError::InvalidByteStringLength { offset });
    }
    let mut length = 0usize;
    for digit in digits {
        length = length
            .checked_mul(10)
            .and_then(|value| value.checked_add(usize::from(digit - b'0')))
            .ok_or(BencodeError::InvalidByteStringLength { offset })?;
    }
    let data_start = cursor + 1;
    let data_end = data_start
        .checked_add(length)
        .ok_or(BencodeError::InvalidByteStringLength { offset })?;
    if data_end > input.len() {
        return Err(BencodeError::ByteStringOutOfBounds {
            offset,
            length,
            remaining: input.len().saturating_sub(data_start),
        });
    }
    Ok(data_end)
}
