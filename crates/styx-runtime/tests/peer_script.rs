use bytes::Bytes;
use styx_disk::{BlockLength, BlockOffset, BlockSpec, PieceIndex};
use styx_proto::{
    read_handshake, read_message, write_handshake, write_message, ExtensionBits, Handshake,
    InfoHashV1, PeerId, PeerMessage, PeerWireError, DEFAULT_MAX_PEER_FRAME_LEN,
};
use styx_runtime::{download_piece_from_peer, PeerPieceRequest, RuntimeError};
use tokio::io::duplex;

#[tokio::test]
async fn download_piece_from_peer_requests_blocks_after_unchoke() {
    let (mut client, mut server) = duplex(4096);
    let info_hash = InfoHashV1::new([1; 20]);
    let local_peer_id = PeerId::new([2; 20]);
    let remote_peer_id = PeerId::new([3; 20]);
    let block = block(0, 0, 4, 4);

    let peer = tokio::spawn(async move {
        let received = read_handshake(&mut server, info_hash).await.unwrap();
        assert_eq!(received.peer_id, local_peer_id);
        write_handshake(
            &mut server,
            &Handshake {
                reserved: ExtensionBits::default(),
                info_hash,
                peer_id: remote_peer_id,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            read_message(&mut server, DEFAULT_MAX_PEER_FRAME_LEN)
                .await
                .unwrap(),
            PeerMessage::Interested
        );
        write_message(&mut server, &PeerMessage::Unchoke)
            .await
            .unwrap();
        assert_eq!(
            read_message(&mut server, DEFAULT_MAX_PEER_FRAME_LEN)
                .await
                .unwrap(),
            PeerMessage::Request {
                index: 0,
                begin: 0,
                length: 4,
            }
        );
        write_message(
            &mut server,
            &PeerMessage::Piece {
                index: 0,
                begin: 0,
                block: Bytes::from_static(b"abcd"),
            },
        )
        .await
        .unwrap();
    });

    let piece = download_piece_from_peer(
        &mut client,
        PeerPieceRequest {
            info_hash,
            local_peer_id,
            target_piece: PieceIndex::new(0),
            blocks: vec![block],
            max_frame_len: DEFAULT_MAX_PEER_FRAME_LEN,
        },
    )
    .await
    .unwrap();

    peer.await.unwrap();
    assert_eq!(piece.blocks, vec![(block, Bytes::from_static(b"abcd"))]);
}

#[tokio::test]
async fn download_piece_from_peer_rejects_handshake_info_hash_mismatch() {
    let (mut client, mut server) = duplex(4096);
    let expected_info_hash = InfoHashV1::new([1; 20]);
    let wrong_info_hash = InfoHashV1::new([9; 20]);
    let local_peer_id = PeerId::new([2; 20]);

    let peer = tokio::spawn(async move {
        let _ = read_handshake(&mut server, expected_info_hash)
            .await
            .unwrap();
        write_handshake(
            &mut server,
            &Handshake {
                reserved: ExtensionBits::default(),
                info_hash: wrong_info_hash,
                peer_id: PeerId::new([3; 20]),
            },
        )
        .await
        .unwrap();
    });

    let err = download_piece_from_peer(
        &mut client,
        PeerPieceRequest {
            info_hash: expected_info_hash,
            local_peer_id,
            target_piece: PieceIndex::new(0),
            blocks: vec![block(0, 0, 4, 4)],
            max_frame_len: DEFAULT_MAX_PEER_FRAME_LEN,
        },
    )
    .await
    .unwrap_err();

    peer.await.unwrap();
    assert!(matches!(
        err,
        RuntimeError::PeerWire(PeerWireError::InfoHashMismatch)
    ));
}

fn block(piece: u32, offset: u32, length: u32, piece_length: u32) -> BlockSpec {
    BlockSpec::new(
        PieceIndex::new(piece),
        BlockOffset::new(offset),
        BlockLength::new(length).unwrap(),
        piece_length,
    )
    .unwrap()
}
