use std::{net::SocketAddr, time::Duration};

use bytes::{Bytes, BytesMut};
use styx_proto::{
    decode_extension_handshake, decode_metadata_message, encode_extension_handshake,
    encode_metadata_message, metadata_piece_count, read_handshake, read_message, write_handshake,
    write_message, ExtensionBits, ExtensionHandshake, InfoHashV1, MetadataMessage, PeerId,
    PeerMessage, METADATA_BLOCK_LEN,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::RuntimeError;

const LOCAL_METADATA_EXTENSION_ID: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetadataFetchConfig {
    pub max_metadata_size: u64,
    pub request_limit: u32,
    pub timeout: Duration,
    pub max_frame_len: u32,
}

impl Default for MetadataFetchConfig {
    fn default() -> Self {
        Self {
            max_metadata_size: 8 * 1024 * 1024,
            request_limit: 512,
            timeout: Duration::from_secs(15),
            max_frame_len: 64 * 1024,
        }
    }
}

impl MetadataFetchConfig {
    pub fn validate(self) -> Result<Self, RuntimeError> {
        if self.max_metadata_size == 0 {
            return Err(RuntimeError::InvalidConfig(
                "metadata max_metadata_size must be greater than zero",
            ));
        }
        if self.request_limit == 0 {
            return Err(RuntimeError::InvalidConfig(
                "metadata request_limit must be greater than zero",
            ));
        }
        if self.timeout.is_zero() {
            return Err(RuntimeError::InvalidConfig(
                "metadata timeout must be greater than zero",
            ));
        }
        if self.max_frame_len == 0 {
            return Err(RuntimeError::InvalidConfig(
                "metadata max_frame_len must be greater than zero",
            ));
        }
        Ok(self)
    }
}

pub async fn fetch_metadata_from_stream<S>(
    stream: &mut S,
    info_hash: InfoHashV1,
    peer_id: PeerId,
    config: MetadataFetchConfig,
) -> Result<Bytes, RuntimeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let config = config.validate()?;
    write_handshake(
        stream,
        &styx_proto::Handshake {
            reserved: ExtensionBits::default().with_extended(),
            info_hash,
            peer_id,
        },
    )
    .await?;

    let peer_handshake = timeout(config.timeout, read_handshake(stream, info_hash))
        .await
        .map_err(|_| RuntimeError::Timeout {
            stage: "reading metadata peer handshake",
        })??;
    if !peer_handshake.reserved.supports_extended() {
        return Err(RuntimeError::Metadata(
            "peer does not advertise extension protocol support".to_owned(),
        ));
    }

    write_message(stream, &local_extension_handshake()).await?;
    let peer_extensions = read_peer_extension_handshake(stream, config).await?;
    let remote_metadata_id = peer_extensions
        .message_id("ut_metadata")
        .ok_or_else(|| RuntimeError::Metadata("peer did not advertise ut_metadata".to_owned()))?;
    let total_size = peer_extensions
        .metadata_size
        .ok_or_else(|| RuntimeError::Metadata("peer omitted metadata_size".to_owned()))?;
    validate_metadata_size(total_size, config)?;

    let piece_count =
        metadata_piece_count(total_size).map_err(|err| RuntimeError::Metadata(err.to_string()))?;
    if piece_count > config.request_limit {
        return Err(RuntimeError::Metadata(
            "metadata request count exceeds configured limit".to_owned(),
        ));
    }

    let mut metadata = BytesMut::with_capacity(total_size as usize);
    for piece in 0..piece_count {
        write_message(
            stream,
            &PeerMessage::Extended {
                extension_id: remote_metadata_id,
                payload: Bytes::from(encode_metadata_message(&MetadataMessage::Request { piece })),
            },
        )
        .await?;
        let payload = read_metadata_piece(stream, piece, total_size, config).await?;
        metadata.extend_from_slice(&payload);
    }

    if metadata.len() as u64 != total_size {
        return Err(RuntimeError::Metadata(
            "assembled metadata length does not match advertised size".to_owned(),
        ));
    }
    Ok(metadata.freeze())
}

pub async fn fetch_metadata_from_peer(
    addr: SocketAddr,
    info_hash: InfoHashV1,
    peer_id: PeerId,
    config: MetadataFetchConfig,
) -> Result<Bytes, RuntimeError> {
    let config = config.validate()?;
    let mut stream = timeout(config.timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| RuntimeError::Timeout {
            stage: "connecting to metadata peer",
        })?
        .map_err(RuntimeError::from)?;
    fetch_metadata_from_stream(&mut stream, info_hash, peer_id, config).await
}

fn local_extension_handshake() -> PeerMessage {
    let mut handshake = ExtensionHandshake::default();
    handshake
        .messages
        .insert("ut_metadata".to_owned(), LOCAL_METADATA_EXTENSION_ID);
    PeerMessage::Extended {
        extension_id: 0,
        payload: Bytes::from(encode_extension_handshake(&handshake)),
    }
}

async fn read_peer_extension_handshake<S>(
    stream: &mut S,
    config: MetadataFetchConfig,
) -> Result<ExtensionHandshake, RuntimeError>
where
    S: AsyncRead + Unpin,
{
    loop {
        let message = read_timed_message(stream, config, "reading extension handshake").await?;
        match message {
            PeerMessage::Extended {
                extension_id: 0,
                payload,
            } => {
                return decode_extension_handshake(&payload)
                    .map_err(|err| RuntimeError::Metadata(err.to_string()));
            }
            PeerMessage::KeepAlive
            | PeerMessage::Have { .. }
            | PeerMessage::Bitfield { .. }
            | PeerMessage::Port { .. } => {}
            _ => {
                return Err(RuntimeError::UnexpectedPeerMessage {
                    stage: "reading extension handshake",
                });
            }
        }
    }
}

async fn read_metadata_piece<S>(
    stream: &mut S,
    expected_piece: u32,
    total_size: u64,
    config: MetadataFetchConfig,
) -> Result<Bytes, RuntimeError>
where
    S: AsyncRead + Unpin,
{
    loop {
        let message = read_timed_message(stream, config, "reading metadata piece").await?;
        match message {
            PeerMessage::Extended {
                extension_id: LOCAL_METADATA_EXTENSION_ID,
                payload,
            } => match decode_metadata_message(&payload)
                .map_err(|err| RuntimeError::Metadata(err.to_string()))?
            {
                MetadataMessage::Data {
                    piece,
                    total_size: observed_size,
                    payload,
                } if piece == expected_piece && observed_size == total_size => {
                    validate_piece_payload(expected_piece, total_size, payload.len())?;
                    return Ok(payload);
                }
                MetadataMessage::Reject { piece } if piece == expected_piece => {
                    return Err(RuntimeError::Metadata(format!(
                        "peer rejected metadata piece {piece}"
                    )));
                }
                _ => {
                    return Err(RuntimeError::UnexpectedPeerMessage {
                        stage: "reading metadata piece",
                    });
                }
            },
            PeerMessage::KeepAlive => {}
            _ => {
                return Err(RuntimeError::UnexpectedPeerMessage {
                    stage: "reading metadata piece",
                });
            }
        }
    }
}

async fn read_timed_message<S>(
    stream: &mut S,
    config: MetadataFetchConfig,
    stage: &'static str,
) -> Result<PeerMessage, RuntimeError>
where
    S: AsyncRead + Unpin,
{
    timeout(config.timeout, read_message(stream, config.max_frame_len))
        .await
        .map_err(|_| RuntimeError::Timeout { stage })?
        .map_err(RuntimeError::from)
}

fn validate_metadata_size(size: u64, config: MetadataFetchConfig) -> Result<(), RuntimeError> {
    if size == 0 {
        return Err(RuntimeError::Metadata(
            "peer advertised empty metadata".to_owned(),
        ));
    }
    if size > config.max_metadata_size {
        return Err(RuntimeError::Metadata(format!(
            "peer metadata_size {size} exceeds configured maximum {}",
            config.max_metadata_size
        )));
    }
    Ok(())
}

fn validate_piece_payload(piece: u32, total_size: u64, len: usize) -> Result<(), RuntimeError> {
    let expected = if (piece as u64 + 1) * METADATA_BLOCK_LEN >= total_size {
        let start = piece as u64 * METADATA_BLOCK_LEN;
        total_size
            .checked_sub(start)
            .ok_or_else(|| RuntimeError::Metadata("metadata piece index out of range".to_owned()))?
            as usize
    } else {
        METADATA_BLOCK_LEN as usize
    };
    if len != expected {
        return Err(RuntimeError::Metadata(format!(
            "metadata piece {piece} had {len} bytes, expected {expected}"
        )));
    }
    Ok(())
}
