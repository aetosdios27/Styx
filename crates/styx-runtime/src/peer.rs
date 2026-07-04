use bytes::Bytes;
use styx_disk::{BlockSpec, PieceIndex};
use styx_proto::{
    read_handshake, read_message, write_handshake, write_message, ExtensionBits, Handshake,
    InfoHashV1, PeerId, PeerMessage,
};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::RuntimeError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerPieceRequest {
    pub info_hash: InfoHashV1,
    pub local_peer_id: PeerId,
    pub target_piece: PieceIndex,
    pub blocks: Vec<BlockSpec>,
    pub max_frame_len: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadedPiece {
    pub piece: PieceIndex,
    pub blocks: Vec<(BlockSpec, Bytes)>,
}

pub async fn download_piece_from_peer<S>(
    stream: &mut S,
    request: PeerPieceRequest,
) -> Result<DownloadedPiece, RuntimeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_handshake(
        stream,
        &Handshake {
            reserved: ExtensionBits::default(),
            info_hash: request.info_hash,
            peer_id: request.local_peer_id,
        },
    )
    .await?;
    read_handshake(stream, request.info_hash).await?;
    write_message(stream, &PeerMessage::Interested).await?;
    wait_for_unchoke(stream, request.max_frame_len).await?;

    let mut blocks = Vec::with_capacity(request.blocks.len());
    for block in request.blocks {
        write_message(
            stream,
            &PeerMessage::Request {
                index: block.piece().get(),
                begin: block.offset().get(),
                length: block.length().get(),
            },
        )
        .await?;
        let payload = wait_for_block(stream, request.max_frame_len, block).await?;
        blocks.push((block, payload));
    }

    Ok(DownloadedPiece {
        piece: request.target_piece,
        blocks,
    })
}

async fn wait_for_unchoke<S>(stream: &mut S, max_frame_len: u32) -> Result<(), RuntimeError>
where
    S: AsyncRead + Unpin,
{
    loop {
        match read_message(stream, max_frame_len).await? {
            PeerMessage::Unchoke => return Ok(()),
            PeerMessage::KeepAlive | PeerMessage::Have { .. } | PeerMessage::Bitfield { .. } => {}
            PeerMessage::Choke => return Err(RuntimeError::PeerChoked),
            _ => {
                return Err(RuntimeError::UnexpectedPeerMessage {
                    stage: "waiting_for_unchoke",
                })
            }
        }
    }
}

async fn wait_for_block<S>(
    stream: &mut S,
    max_frame_len: u32,
    expected: BlockSpec,
) -> Result<Bytes, RuntimeError>
where
    S: AsyncRead + Unpin,
{
    loop {
        match read_message(stream, max_frame_len).await? {
            PeerMessage::Piece {
                index,
                begin,
                block,
            } if index == expected.piece().get()
                && begin == expected.offset().get()
                && block.len() == expected.length().get() as usize =>
            {
                return Ok(block)
            }
            PeerMessage::KeepAlive | PeerMessage::Have { .. } | PeerMessage::Bitfield { .. } => {}
            PeerMessage::Choke => return Err(RuntimeError::PeerChoked),
            _ => {
                return Err(RuntimeError::UnexpectedPeerMessage {
                    stage: "waiting_for_piece_block",
                })
            }
        }
    }
}
