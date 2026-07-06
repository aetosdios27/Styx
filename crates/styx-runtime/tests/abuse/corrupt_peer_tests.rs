use bytes::Bytes;
use std::time::Duration;
use styx_disk::{BlockLength, BlockOffset, BlockSpec, PieceIndex};
use styx_proto::{
    ExtensionBits, Handshake, InfoHashV1, PeerId, PeerMessage, PeerWireError,
    DEFAULT_MAX_PEER_FRAME_LEN,
};
use styx_runtime::{download_piece_from_peer, PeerPieceRequest, RuntimeError};
use tokio::io::duplex;

use crate::abuse::mock_peer::{MockPeerBuilder, ScriptedEvent};

#[tokio::test]
async fn corrupt_peer_bit_flipped_piece_returns_error() {
    let (mut client, server) = duplex(4096);
    let info_hash = InfoHashV1::new([1; 20]);
    let local_peer_id = PeerId::new([2; 20]);
    let remote_peer_id = PeerId::new([3; 20]);

    let mock = MockPeerBuilder::new(Handshake {
        reserved: ExtensionBits::default(),
        info_hash,
        peer_id: remote_peer_id,
    })
    .add_event(ScriptedEvent::Receive {
        expect: PeerMessage::Interested,
        timeout: Duration::from_secs(5),
    })
    .add_event(ScriptedEvent::Send(PeerMessage::Unchoke))
    .add_event(ScriptedEvent::Receive {
        expect: PeerMessage::Request {
            index: 0,
            begin: 0,
            length: 4,
        },
        timeout: Duration::from_secs(5),
    })
    .add_event(ScriptedEvent::CorruptNext(1, 0))
    .add_event(ScriptedEvent::Send(PeerMessage::Piece {
        index: 0,
        begin: 0,
        block: Bytes::from_static(b"abcd"),
    }))
    .build();

    let peer_handle = tokio::spawn(async move { mock.run(server).await });

    let err = download_piece_from_peer(
        &mut client,
        PeerPieceRequest {
            info_hash,
            local_peer_id,
            target_piece: PieceIndex::new(0),
            blocks: vec![block(0, 0, 4, 4)],
            max_frame_len: DEFAULT_MAX_PEER_FRAME_LEN,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            err,
            RuntimeError::PeerWire(PeerWireError::FrameTooLarge { .. })
        ),
        "expected FrameTooLarge, got {err:?}"
    );
    peer_handle.abort();
}

#[tokio::test]
async fn corrupt_peer_wrong_index_in_piece_returns_error() {
    let (mut client, server) = duplex(4096);
    let info_hash = InfoHashV1::new([1; 20]);
    let local_peer_id = PeerId::new([2; 20]);
    let remote_peer_id = PeerId::new([3; 20]);

    let mock = MockPeerBuilder::new(Handshake {
        reserved: ExtensionBits::default(),
        info_hash,
        peer_id: remote_peer_id,
    })
    .add_event(ScriptedEvent::Receive {
        expect: PeerMessage::Interested,
        timeout: Duration::from_secs(5),
    })
    .add_event(ScriptedEvent::Send(PeerMessage::Unchoke))
    .add_event(ScriptedEvent::Receive {
        expect: PeerMessage::Request {
            index: 0,
            begin: 0,
            length: 4,
        },
        timeout: Duration::from_secs(5),
    })
    .add_event(ScriptedEvent::Send(PeerMessage::Piece {
        index: 1,
        begin: 0,
        block: Bytes::from_static(b"abcd"),
    }))
    .build();

    let peer_handle = tokio::spawn(async move { mock.run(server).await });

    let err = download_piece_from_peer(
        &mut client,
        PeerPieceRequest {
            info_hash,
            local_peer_id,
            target_piece: PieceIndex::new(0),
            blocks: vec![block(0, 0, 4, 4)],
            max_frame_len: DEFAULT_MAX_PEER_FRAME_LEN,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            err,
            RuntimeError::UnexpectedPeerMessage {
                stage: "waiting_for_piece_block"
            }
        ),
        "expected UnexpectedPeerMessage, got {err:?}"
    );
    peer_handle.abort();
}

#[tokio::test]
async fn corrupt_peer_garbage_frames_disconnects() {
    let (mut client, server) = duplex(4096);
    let info_hash = InfoHashV1::new([1; 20]);
    let local_peer_id = PeerId::new([2; 20]);
    let remote_peer_id = PeerId::new([3; 20]);

    let mock = MockPeerBuilder::new(Handshake {
        reserved: ExtensionBits::default(),
        info_hash,
        peer_id: remote_peer_id,
    })
    .add_event(ScriptedEvent::Receive {
        expect: PeerMessage::Interested,
        timeout: Duration::from_secs(5),
    })
    .add_event(ScriptedEvent::SendRaw(vec![0x00, 0x00, 0x00, 0x01, 0xAA]))
    .build();

    let peer_handle = tokio::spawn(async move { mock.run(server).await });

    let err = download_piece_from_peer(
        &mut client,
        PeerPieceRequest {
            info_hash,
            local_peer_id,
            target_piece: PieceIndex::new(0),
            blocks: vec![block(0, 0, 4, 4)],
            max_frame_len: DEFAULT_MAX_PEER_FRAME_LEN,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, RuntimeError::PeerWire(_)),
        "expected PeerWire error, got {err:?}"
    );
    peer_handle.abort();
}

#[tokio::test]
async fn corrupt_peer_handshake_then_stall_times_out() {
    let (mut client, server) = duplex(4096);
    let info_hash = InfoHashV1::new([1; 20]);
    let local_peer_id = PeerId::new([2; 20]);
    let remote_peer_id = PeerId::new([3; 20]);

    let mock = MockPeerBuilder::new(Handshake {
        reserved: ExtensionBits::default(),
        info_hash,
        peer_id: remote_peer_id,
    })
    .add_event(ScriptedEvent::Stall(Duration::from_secs(600)))
    .build();

    let peer_handle = tokio::spawn(async move { mock.run(server).await });

    let result = tokio::time::timeout(
        Duration::from_millis(100),
        download_piece_from_peer(
            &mut client,
            PeerPieceRequest {
                info_hash,
                local_peer_id,
                target_piece: PieceIndex::new(0),
                blocks: vec![block(0, 0, 4, 4)],
                max_frame_len: DEFAULT_MAX_PEER_FRAME_LEN,
            },
        ),
    )
    .await;

    assert!(result.is_err(), "expected timeout, got {:?}", result);
    peer_handle.abort();
}

#[tokio::test]
async fn corrupt_peer_bitfield_then_bad_data_succeeds_with_corrupted_payload() {
    let (mut client, server) = duplex(4096);
    let info_hash = InfoHashV1::new([1; 20]);
    let local_peer_id = PeerId::new([2; 20]);
    let remote_peer_id = PeerId::new([3; 20]);

    let mock = MockPeerBuilder::new(Handshake {
        reserved: ExtensionBits::default(),
        info_hash,
        peer_id: remote_peer_id,
    })
    .add_event(ScriptedEvent::Receive {
        expect: PeerMessage::Interested,
        timeout: Duration::from_secs(5),
    })
    .add_event(ScriptedEvent::Send(PeerMessage::Bitfield {
        bytes: Bytes::from_static(&[0x80]),
    }))
    .add_event(ScriptedEvent::Send(PeerMessage::Unchoke))
    .add_event(ScriptedEvent::Receive {
        expect: PeerMessage::Request {
            index: 0,
            begin: 0,
            length: 4,
        },
        timeout: Duration::from_secs(5),
    })
    .add_event(ScriptedEvent::Send(PeerMessage::Piece {
        index: 0,
        begin: 0,
        block: Bytes::from_static(b"XXXX"),
    }))
    .build();

    let peer_handle = tokio::spawn(async move { mock.run(server).await });

    let piece = download_piece_from_peer(
        &mut client,
        PeerPieceRequest {
            info_hash,
            local_peer_id,
            target_piece: PieceIndex::new(0),
            blocks: vec![block(0, 0, 4, 4)],
            max_frame_len: DEFAULT_MAX_PEER_FRAME_LEN,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        piece.blocks[0].1,
        Bytes::from_static(b"XXXX"),
        "expected corrupted block data"
    );
    assert_ne!(
        piece.blocks[0].1,
        Bytes::from_static(b"abcd"),
        "data should NOT match the valid content"
    );
    peer_handle.await.unwrap();
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
