//! BEP 3 peer-wire handshake and message framing.

use bytes::{BufMut, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::metainfo::InfoHashV1;

const PROTOCOL_STRING: &[u8; 19] = b"BitTorrent protocol";
const PROTOCOL_STRING_LEN: u8 = 19;

/// Length in bytes of a v1 BitTorrent peer handshake.
pub const PEER_HANDSHAKE_LEN: usize = 68;
/// Standard BitTorrent request block length, 16 KiB.
pub const DEFAULT_BLOCK_LEN: u32 = 16 * 1024;
/// Default maximum length-prefixed peer message payload accepted by readers.
pub const DEFAULT_MAX_PEER_FRAME_LEN: u32 = DEFAULT_BLOCK_LEN + 9;

/// A 20-byte peer identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PeerId([u8; 20]);

impl PeerId {
    /// Construct a peer id from raw bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    /// Return the raw 20-byte peer id.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }
}

/// Reserved extension bits carried in the BitTorrent handshake.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ExtensionBits([u8; 8]);

impl ExtensionBits {
    /// Construct extension bits from raw handshake bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }

    /// Return the raw 8-byte extension bitfield.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }
}

/// A decoded BEP 3 peer handshake.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Handshake {
    /// Reserved extension bits.
    pub reserved: ExtensionBits,
    /// Expected v1 torrent info hash.
    pub info_hash: InfoHashV1,
    /// Remote peer identifier.
    pub peer_id: PeerId,
}

use crate::hash_msg;

/// A decoded length-prefixed BEP 3 peer message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerMessage {
    /// Zero-length keepalive frame.
    KeepAlive,
    /// Message id 0.
    Choke,
    /// Message id 1.
    Unchoke,
    /// Message id 2.
    Interested,
    /// Message id 3.
    NotInterested,
    /// Message id 4.
    Have {
        /// Piece index advertised by the peer.
        piece_index: u32,
    },
    /// Message id 5.
    Bitfield {
        /// Raw bitfield bytes.
        bytes: Bytes,
    },
    /// Message id 6.
    Request {
        /// Piece index.
        index: u32,
        /// Byte offset within the piece.
        begin: u32,
        /// Requested block length.
        length: u32,
    },
    /// Message id 7.
    Piece {
        /// Piece index.
        index: u32,
        /// Byte offset within the piece.
        begin: u32,
        /// Block payload.
        block: Bytes,
    },
    /// Message id 8.
    Cancel {
        /// Piece index.
        index: u32,
        /// Byte offset within the piece.
        begin: u32,
        /// Canceled block length.
        length: u32,
    },
    /// Message id 21 (BEP 52) — hash request.
    HashRequest(Box<hash_msg::HashRequest>),
    /// Message id 22 (BEP 52) — hashes response.
    Hashes(Box<hash_msg::HashesMessage>),
    /// Message id 23 (BEP 52) — hash reject.
    HashReject(Box<hash_msg::HashReject>),
}

/// Errors returned while encoding or decoding peer-wire data.
#[derive(Debug, thiserror::Error)]
pub enum PeerWireError {
    /// Handshake byte count was not the v1 BEP 3 length.
    #[error("invalid handshake length: expected {expected} bytes, got {actual}")]
    InvalidHandshakeLength {
        /// Expected byte count.
        expected: usize,
        /// Actual byte count.
        actual: usize,
    },
    /// Handshake did not contain the BEP 3 protocol string.
    #[error("invalid BitTorrent protocol string in handshake")]
    InvalidProtocolString,
    /// The received info hash did not match the torrent session.
    #[error("handshake info hash mismatch")]
    InfoHashMismatch,
    /// The frame did not contain a complete 4-byte length prefix.
    #[error("truncated peer message length prefix: got {actual} bytes")]
    TruncatedLengthPrefix {
        /// Available bytes.
        actual: usize,
    },
    /// The frame length exceeded the configured cap.
    #[error("peer message frame length {length} exceeds maximum {max}")]
    FrameTooLarge {
        /// Declared payload length.
        length: u32,
        /// Configured maximum payload length.
        max: u32,
    },
    /// The input ended before the declared message payload length.
    #[error("truncated peer message frame: declared {declared} payload bytes, got {actual}")]
    TruncatedFrame {
        /// Declared payload length.
        declared: u32,
        /// Available payload bytes.
        actual: usize,
    },
    /// The input contained bytes after the declared frame payload.
    #[error(
        "peer message frame has trailing data: declared {declared} payload bytes, got {actual}"
    )]
    TrailingFrameBytes {
        /// Declared payload length.
        declared: u32,
        /// Available payload bytes.
        actual: usize,
    },
    /// A known message id carried an invalid payload length.
    #[error("invalid {message} message length: expected {expected}, got {actual}")]
    InvalidMessageLength {
        /// Message name.
        message: &'static str,
        /// Expected payload length description.
        expected: &'static str,
        /// Actual payload length in bytes.
        actual: usize,
    },
    /// A request-like message declared a zero block length.
    #[error("{message} message length field must be greater than zero")]
    ZeroBlockLength {
        /// Message name.
        message: &'static str,
    },
    /// An outbound message is too large for the peer-wire length prefix.
    #[error("outbound peer message payload length {length} exceeds u32 length prefix")]
    PayloadLengthOverflow {
        /// Payload length in bytes.
        length: usize,
    },
    /// A message id is not defined by BEP 3.
    #[error("unknown peer message id {id}")]
    UnknownMessageId {
        /// Unknown id byte.
        id: u8,
    },
    /// Underlying async IO failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Encode a v1 peer handshake.
#[must_use]
pub fn encode_handshake(handshake: &Handshake) -> Bytes {
    let mut bytes = BytesMut::with_capacity(PEER_HANDSHAKE_LEN);
    bytes.put_u8(PROTOCOL_STRING_LEN);
    bytes.extend_from_slice(PROTOCOL_STRING);
    bytes.extend_from_slice(handshake.reserved.as_bytes());
    bytes.extend_from_slice(handshake.info_hash.as_bytes());
    bytes.extend_from_slice(handshake.peer_id.as_bytes());
    bytes.freeze()
}

/// Decode a v1 peer handshake without validating the expected torrent hash.
///
/// # Errors
///
/// Returns [`PeerWireError`] when the input is not exactly one BEP 3 v1
/// handshake or carries the wrong protocol identifier.
pub fn decode_handshake(input: &[u8]) -> Result<Handshake, PeerWireError> {
    if input.len() != PEER_HANDSHAKE_LEN {
        return Err(PeerWireError::InvalidHandshakeLength {
            expected: PEER_HANDSHAKE_LEN,
            actual: input.len(),
        });
    }
    if input[0] != PROTOCOL_STRING_LEN || &input[1..20] != PROTOCOL_STRING {
        return Err(PeerWireError::InvalidProtocolString);
    }

    let mut reserved = [0; 8];
    reserved.copy_from_slice(&input[20..28]);
    let mut info_hash = [0; 20];
    info_hash.copy_from_slice(&input[28..48]);
    let mut peer_id = [0; 20];
    peer_id.copy_from_slice(&input[48..68]);

    Ok(Handshake {
        reserved: ExtensionBits::new(reserved),
        info_hash: InfoHashV1::new(info_hash),
        peer_id: PeerId::new(peer_id),
    })
}

/// Decode a v1 peer handshake and validate it belongs to the expected torrent.
///
/// # Errors
///
/// Returns [`PeerWireError::InfoHashMismatch`] when the received info hash does
/// not match `expected_info_hash`; other malformed input returns the relevant
/// handshake parse error.
pub fn validate_handshake(
    input: &[u8],
    expected_info_hash: InfoHashV1,
) -> Result<Handshake, PeerWireError> {
    let handshake = decode_handshake(input)?;
    if handshake.info_hash != expected_info_hash {
        return Err(PeerWireError::InfoHashMismatch);
    }
    Ok(handshake)
}

/// Encode a length-prefixed peer message.
///
/// # Errors
///
/// Returns [`PeerWireError`] if `message` represents an invalid outbound frame,
/// such as a zero-length request or an empty piece block.
pub fn encode_message(message: &PeerMessage) -> Result<Bytes, PeerWireError> {
    validate_outbound_message(message)?;
    let payload_len = message_payload_len(message);
    let payload_len_u32 =
        u32::try_from(payload_len).map_err(|_| PeerWireError::PayloadLengthOverflow {
            length: payload_len,
        })?;
    let mut bytes = BytesMut::with_capacity(4 + payload_len);
    bytes.put_u32(payload_len_u32);
    match message {
        PeerMessage::KeepAlive => {}
        PeerMessage::Choke => bytes.put_u8(0),
        PeerMessage::Unchoke => bytes.put_u8(1),
        PeerMessage::Interested => bytes.put_u8(2),
        PeerMessage::NotInterested => bytes.put_u8(3),
        PeerMessage::Have { piece_index } => {
            bytes.put_u8(4);
            bytes.put_u32(*piece_index);
        }
        PeerMessage::Bitfield { bytes: bitfield } => {
            bytes.put_u8(5);
            bytes.extend_from_slice(bitfield);
        }
        PeerMessage::Request {
            index,
            begin,
            length,
        } => {
            bytes.put_u8(6);
            put_request_payload(&mut bytes, *index, *begin, *length);
        }
        PeerMessage::Piece {
            index,
            begin,
            block,
        } => {
            bytes.put_u8(7);
            bytes.put_u32(*index);
            bytes.put_u32(*begin);
            bytes.extend_from_slice(block);
        }
        PeerMessage::Cancel {
            index,
            begin,
            length,
        } => {
            bytes.put_u8(8);
            put_request_payload(&mut bytes, *index, *begin, *length);
        }
        PeerMessage::HashRequest(req) => {
            let encoded = req.encode();
            bytes.extend_from_slice(&encoded);
        }
        PeerMessage::Hashes(h) => {
            let encoded = h.encode();
            bytes.extend_from_slice(&encoded);
        }
        PeerMessage::HashReject(rej) => {
            let encoded = rej.encode();
            bytes.extend_from_slice(&encoded);
        }
    }
    Ok(bytes.freeze())
}

/// Decode a complete length-prefixed peer message using the default frame cap.
///
/// # Errors
///
/// Returns [`PeerWireError`] for truncated, oversized, trailing, unknown, or
/// semantically invalid message frames.
pub fn decode_message_frame(input: &[u8]) -> Result<PeerMessage, PeerWireError> {
    decode_message_frame_with_limit(input, DEFAULT_MAX_PEER_FRAME_LEN)
}

/// Decode a complete length-prefixed peer message using a caller-supplied cap.
///
/// # Errors
///
/// Returns [`PeerWireError`] for truncated, oversized, trailing, unknown, or
/// semantically invalid message frames.
pub fn decode_message_frame_with_limit(
    input: &[u8],
    max_payload_len: u32,
) -> Result<PeerMessage, PeerWireError> {
    let payload = complete_payload(input, max_payload_len)?;
    decode_message_payload(payload)
}

/// Read, decode, and validate a v1 peer handshake from an async stream.
///
/// # Errors
///
/// Returns IO errors from the stream, malformed handshake errors, or
/// [`PeerWireError::InfoHashMismatch`] when the session hash differs.
pub async fn read_handshake<R>(
    reader: &mut R,
    expected_info_hash: InfoHashV1,
) -> Result<Handshake, PeerWireError>
where
    R: AsyncRead + Unpin,
{
    let mut bytes = [0; PEER_HANDSHAKE_LEN];
    reader.read_exact(&mut bytes).await?;
    validate_handshake(&bytes, expected_info_hash)
}

/// Encode and write a v1 peer handshake to an async stream.
///
/// # Errors
///
/// Returns IO errors from the stream.
pub async fn write_handshake<W>(writer: &mut W, handshake: &Handshake) -> Result<(), PeerWireError>
where
    W: AsyncWrite + Unpin,
{
    let encoded = encode_handshake(handshake);
    writer.write_all(&encoded).await?;
    Ok(())
}

/// Read and decode one length-prefixed peer message from an async stream.
///
/// # Errors
///
/// Returns IO errors from the stream or [`PeerWireError`] for oversized or
/// malformed message payloads.
pub async fn read_message<R>(
    reader: &mut R,
    max_payload_len: u32,
) -> Result<PeerMessage, PeerWireError>
where
    R: AsyncRead + Unpin,
{
    let mut len_prefix = [0; 4];
    reader.read_exact(&mut len_prefix).await?;
    let payload_len = u32::from_be_bytes(len_prefix);
    if payload_len > max_payload_len {
        return Err(PeerWireError::FrameTooLarge {
            length: payload_len,
            max: max_payload_len,
        });
    }
    if payload_len == 0 {
        return Ok(PeerMessage::KeepAlive);
    }

    let mut payload = vec![0; payload_len as usize];
    reader.read_exact(&mut payload).await?;
    decode_message_payload(&payload)
}

/// Encode and write one length-prefixed peer message to an async stream.
///
/// # Errors
///
/// Returns IO errors from the stream.
pub async fn write_message<W>(writer: &mut W, message: &PeerMessage) -> Result<(), PeerWireError>
where
    W: AsyncWrite + Unpin,
{
    let encoded = encode_message(message)?;
    writer.write_all(&encoded).await?;
    Ok(())
}

fn validate_outbound_message(message: &PeerMessage) -> Result<(), PeerWireError> {
    match message {
        PeerMessage::Request { length, .. } if *length == 0 => {
            Err(PeerWireError::ZeroBlockLength { message: "request" })
        }
        PeerMessage::Cancel { length, .. } if *length == 0 => {
            Err(PeerWireError::ZeroBlockLength { message: "cancel" })
        }
        PeerMessage::Piece { block, .. } if block.is_empty() => {
            Err(PeerWireError::InvalidMessageLength {
                message: "piece",
                expected: "at least 10",
                actual: 9,
            })
        }
        PeerMessage::Hashes(h) if h.hashes.is_empty() => Err(PeerWireError::InvalidMessageLength {
            message: "hashes",
            expected: "at least 1 hash",
            actual: 0,
        }),
        _ => Ok(()),
    }
}

fn message_payload_len(message: &PeerMessage) -> usize {
    match message {
        PeerMessage::KeepAlive => 0,
        PeerMessage::Choke
        | PeerMessage::Unchoke
        | PeerMessage::Interested
        | PeerMessage::NotInterested => 1,
        PeerMessage::Have { .. } => 5,
        PeerMessage::Bitfield { bytes } => 1 + bytes.len(),
        PeerMessage::Request { .. } | PeerMessage::Cancel { .. } => 13,
        PeerMessage::Piece { block, .. } => 9 + block.len(),
        PeerMessage::HashRequest(_) => 1 + 32 + 16,
        PeerMessage::Hashes(h) => 1 + 32 + 16 + h.hashes.len() * 32,
        PeerMessage::HashReject(_) => 1 + 32 + 16,
    }
}

fn complete_payload(input: &[u8], max_payload_len: u32) -> Result<&[u8], PeerWireError> {
    if input.len() < 4 {
        return Err(PeerWireError::TruncatedLengthPrefix {
            actual: input.len(),
        });
    }

    let payload_len = u32::from_be_bytes([input[0], input[1], input[2], input[3]]);
    if payload_len > max_payload_len {
        return Err(PeerWireError::FrameTooLarge {
            length: payload_len,
            max: max_payload_len,
        });
    }

    let available = input.len() - 4;
    let declared = payload_len as usize;
    if available < declared {
        return Err(PeerWireError::TruncatedFrame {
            declared: payload_len,
            actual: available,
        });
    }
    if available > declared {
        return Err(PeerWireError::TrailingFrameBytes {
            declared: payload_len,
            actual: available,
        });
    }

    Ok(&input[4..])
}

fn decode_message_payload(payload: &[u8]) -> Result<PeerMessage, PeerWireError> {
    let Some((&id, body)) = payload.split_first() else {
        return Ok(PeerMessage::KeepAlive);
    };

    match id {
        0 => decode_empty_body("choke", body, PeerMessage::Choke),
        1 => decode_empty_body("unchoke", body, PeerMessage::Unchoke),
        2 => decode_empty_body("interested", body, PeerMessage::Interested),
        3 => decode_empty_body("not_interested", body, PeerMessage::NotInterested),
        4 => {
            require_len("have", body, 4)?;
            Ok(PeerMessage::Have {
                piece_index: read_u32(&body[0..4]),
            })
        }
        5 => Ok(PeerMessage::Bitfield {
            bytes: Bytes::copy_from_slice(body),
        }),
        6 => decode_request_like("request", body).map(|(index, begin, length)| {
            PeerMessage::Request {
                index,
                begin,
                length,
            }
        }),
        7 => {
            if body.len() < 9 {
                return Err(PeerWireError::InvalidMessageLength {
                    message: "piece",
                    expected: "at least 10",
                    actual: body.len() + 1,
                });
            }
            let block = &body[8..];
            Ok(PeerMessage::Piece {
                index: read_u32(&body[0..4]),
                begin: read_u32(&body[4..8]),
                block: Bytes::copy_from_slice(block),
            })
        }
        8 => {
            decode_request_like("cancel", body).map(|(index, begin, length)| PeerMessage::Cancel {
                index,
                begin,
                length,
            })
        }
        21 => {
            let payload = [&[hash_msg::HASH_REQUEST_ID], body].concat();
            hash_msg::HashRequest::decode(&payload)
                .map(|req| PeerMessage::HashRequest(Box::new(req)))
                .map_err(|_| PeerWireError::UnknownMessageId { id: 21 })
        }
        22 => {
            let payload = [&[hash_msg::HASHES_ID], body].concat();
            hash_msg::HashesMessage::decode(&payload)
                .map(|h| PeerMessage::Hashes(Box::new(h)))
                .map_err(|_| PeerWireError::UnknownMessageId { id: 22 })
        }
        23 => {
            let payload = [&[hash_msg::HASH_REJECT_ID], body].concat();
            hash_msg::HashRequest::decode(&payload)
                .map(|rej| PeerMessage::HashReject(Box::new(rej)))
                .map_err(|_| PeerWireError::UnknownMessageId { id: 23 })
        }
        _ => Err(PeerWireError::UnknownMessageId { id }),
    }
}

fn decode_empty_body(
    message: &'static str,
    body: &[u8],
    decoded: PeerMessage,
) -> Result<PeerMessage, PeerWireError> {
    require_len(message, body, 0)?;
    Ok(decoded)
}

fn decode_request_like(
    message: &'static str,
    body: &[u8],
) -> Result<(u32, u32, u32), PeerWireError> {
    require_len(message, body, 12)?;
    let length = read_u32(&body[8..12]);
    if length == 0 {
        return Err(PeerWireError::ZeroBlockLength { message });
    }
    Ok((read_u32(&body[0..4]), read_u32(&body[4..8]), length))
}

fn require_len(message: &'static str, body: &[u8], expected: usize) -> Result<(), PeerWireError> {
    if body.len() == expected {
        return Ok(());
    }
    Err(PeerWireError::InvalidMessageLength {
        message,
        expected: fixed_len_label(expected),
        actual: body.len() + 1,
    })
}

fn fixed_len_label(expected: usize) -> &'static str {
    match expected {
        0 => "1",
        4 => "5",
        12 => "13",
        _ => "fixed",
    }
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn put_request_payload(bytes: &mut BytesMut, index: u32, begin: u32, length: u32) {
    bytes.put_u32(index);
    bytes.put_u32(begin);
    bytes.put_u32(length);
}

#[cfg(test)]
mod tests {
    use proptest::collection::vec as prop_vec;
    use proptest::prelude::*;
    use tokio::io::duplex;

    use super::*;

    fn info_hash(byte: u8) -> InfoHashV1 {
        InfoHashV1::new([byte; 20])
    }

    fn peer_id(byte: u8) -> PeerId {
        PeerId::new([byte; 20])
    }

    fn handshake() -> Handshake {
        Handshake {
            reserved: ExtensionBits::new([0, 0, 0, 0, 0, 0, 0, 1]),
            info_hash: info_hash(7),
            peer_id: peer_id(9),
        }
    }

    #[test]
    fn encode_handshake_returns_bep3_v1_layout() {
        let encoded = encode_handshake(&handshake());

        assert_eq!(encoded.len(), PEER_HANDSHAKE_LEN);
        assert_eq!(encoded[0], PROTOCOL_STRING_LEN);
        assert_eq!(&encoded[1..20], PROTOCOL_STRING);
        assert_eq!(&encoded[28..48], info_hash(7).as_bytes());
    }

    #[test]
    fn decode_handshake_rejects_wrong_length() {
        let err = decode_handshake(&[0; PEER_HANDSHAKE_LEN - 1]).unwrap_err();

        assert!(matches!(
            err,
            PeerWireError::InvalidHandshakeLength { actual: 67, .. }
        ));
    }

    #[test]
    fn decode_handshake_rejects_wrong_protocol_string() {
        let mut encoded = encode_handshake(&handshake()).to_vec();
        encoded[1] = b'X';

        let err = decode_handshake(&encoded).unwrap_err();

        assert!(matches!(err, PeerWireError::InvalidProtocolString));
    }

    #[test]
    fn validate_handshake_rejects_info_hash_mismatch() {
        let encoded = encode_handshake(&handshake());

        let err = validate_handshake(&encoded, info_hash(8)).unwrap_err();

        assert!(matches!(err, PeerWireError::InfoHashMismatch));
    }

    #[test]
    fn validate_handshake_accepts_matching_info_hash() {
        let encoded = encode_handshake(&handshake());

        let decoded = validate_handshake(&encoded, info_hash(7)).unwrap();

        assert_eq!(decoded, handshake());
    }

    #[test]
    fn encode_message_writes_keepalive_as_zero_length_prefix() {
        let encoded = encode_message(&PeerMessage::KeepAlive);

        assert_eq!(encoded.unwrap().as_ref(), &[0, 0, 0, 0]);
    }

    #[test]
    fn decode_message_frame_rejects_truncated_length_prefix() {
        let err = decode_message_frame(&[0, 0, 0]).unwrap_err();

        assert!(matches!(
            err,
            PeerWireError::TruncatedLengthPrefix { actual: 3 }
        ));
    }

    #[test]
    fn decode_message_frame_rejects_truncated_payload() {
        let err = decode_message_frame(&[0, 0, 0, 5, 4, 0, 0]).unwrap_err();

        assert!(matches!(
            err,
            PeerWireError::TruncatedFrame {
                declared: 5,
                actual: 3
            }
        ));
    }

    #[test]
    fn decode_message_frame_rejects_trailing_payload_bytes() {
        let err = decode_message_frame(&[0, 0, 0, 1, 0, 99]).unwrap_err();

        assert!(matches!(
            err,
            PeerWireError::TrailingFrameBytes {
                declared: 1,
                actual: 2
            }
        ));
    }

    #[test]
    fn decode_message_frame_rejects_oversized_payload_before_allocation() {
        let err = decode_message_frame_with_limit(&[0, 0, 0, 10], 9).unwrap_err();

        assert!(matches!(
            err,
            PeerWireError::FrameTooLarge { length: 10, max: 9 }
        ));
    }

    #[test]
    fn decode_message_frame_rejects_unknown_message_id() {
        let err = decode_message_frame(&[0, 0, 0, 1, 99]).unwrap_err();

        assert!(matches!(err, PeerWireError::UnknownMessageId { id: 99 }));
    }

    #[test]
    fn decode_message_frame_rejects_invalid_fixed_size_message_length() {
        let err = decode_message_frame(&[0, 0, 0, 2, 0, 1]).unwrap_err();

        assert!(matches!(
            err,
            PeerWireError::InvalidMessageLength {
                message: "choke",
                ..
            }
        ));
    }

    #[test]
    fn encode_message_rejects_zero_length_request() {
        let message = PeerMessage::Request {
            index: 1,
            begin: 2,
            length: 0,
        };

        let err = encode_message(&message).unwrap_err();

        assert!(matches!(
            err,
            PeerWireError::ZeroBlockLength { message: "request" }
        ));
    }

    #[test]
    fn decode_message_frame_rejects_empty_piece_block() {
        let encoded = [0, 0, 0, 9, 7, 0, 0, 0, 1, 0, 0, 0, 2];

        let err = decode_message_frame(&encoded).unwrap_err();

        assert!(matches!(
            err,
            PeerWireError::InvalidMessageLength {
                message: "piece",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn async_handshake_helpers_handle_fragmented_streams() {
        let expected = handshake();
        let (mut client, mut server) = duplex(8);

        let writer = tokio::spawn(async move { write_handshake(&mut client, &expected).await });
        let decoded = read_handshake(&mut server, info_hash(7)).await.unwrap();
        writer.await.unwrap().unwrap();

        assert_eq!(decoded, expected);
    }

    #[tokio::test]
    async fn async_message_helpers_handle_fragmented_streams() {
        let expected = PeerMessage::Piece {
            index: 3,
            begin: 16,
            block: Bytes::from_static(b"abcd"),
        };
        let (mut client, mut server) = duplex(3);

        let outbound = expected.clone();
        let writer = tokio::spawn(async move { write_message(&mut client, &outbound).await });
        let decoded = read_message(&mut server, DEFAULT_MAX_PEER_FRAME_LEN)
            .await
            .unwrap();
        writer.await.unwrap().unwrap();

        assert_eq!(decoded, expected);
    }

    proptest! {
        #[test]
        fn peer_messages_roundtrip(message in arb_peer_message()) {
            let encoded = encode_message(&message);
            let decoded = decode_message_frame(&encoded.unwrap()).unwrap();
            prop_assert_eq!(decoded, message);
        }

        #[test]
        fn arbitrary_frames_never_panic(input in prop_vec(any::<u8>(), 0..128)) {
            let _ = decode_message_frame(&input);
        }
    }

    fn arb_peer_message() -> impl Strategy<Value = PeerMessage> {
        prop_oneof![
            Just(PeerMessage::KeepAlive),
            Just(PeerMessage::Choke),
            Just(PeerMessage::Unchoke),
            Just(PeerMessage::Interested),
            Just(PeerMessage::NotInterested),
            any::<u32>().prop_map(|piece_index| PeerMessage::Have { piece_index }),
            prop_vec(any::<u8>(), 0..32).prop_map(|bytes| PeerMessage::Bitfield {
                bytes: Bytes::from(bytes)
            }),
            (any::<u32>(), any::<u32>(), 1_u32..=DEFAULT_BLOCK_LEN).prop_map(
                |(index, begin, length)| PeerMessage::Request {
                    index,
                    begin,
                    length,
                }
            ),
            (any::<u32>(), any::<u32>(), prop_vec(any::<u8>(), 1..64)).prop_map(
                |(index, begin, block)| PeerMessage::Piece {
                    index,
                    begin,
                    block: Bytes::from(block),
                }
            ),
            (any::<u32>(), any::<u32>(), 1_u32..=DEFAULT_BLOCK_LEN).prop_map(
                |(index, begin, length)| PeerMessage::Cancel {
                    index,
                    begin,
                    length,
                }
            ),
        ]
    }
}
