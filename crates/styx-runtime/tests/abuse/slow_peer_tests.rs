use std::time::Duration;
use bytes::Bytes;
use styx_disk::{BlockLength, BlockOffset, BlockSpec, PieceIndex};
use styx_proto::{
    ExtensionBits, Handshake, InfoHashV1, PeerId, PeerMessage, DEFAULT_MAX_PEER_FRAME_LEN,
};
use styx_runtime::{download_piece_from_peer, PeerPieceRequest};
use tokio::io::duplex;

use crate::abuse::mock_peer::{MockPeerBuilder, ScriptedEvent};

#[tokio::test]
async fn slow_peer_trickle_data_times_out() {
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
async fn slow_peer_stalls_mid_piece_then_continues() {
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
    .add_event(ScriptedEvent::Stall(Duration::from_millis(200)))
    .add_event(ScriptedEvent::Send(PeerMessage::Piece {
        index: 0,
        begin: 0,
        block: Bytes::from_static(b"abcd"),
    }))
    .build();

    let peer_handle = tokio::spawn(async move { mock.run(server).await });

    let piece = tokio::time::timeout(
        Duration::from_secs(5),
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
    .await
    .expect("download should complete within timeout")
    .expect("download should succeed");

    assert_eq!(
        piece.blocks[0].1,
        Bytes::from_static(b"abcd"),
        "expected valid block data after mid-piece stall"
    );
    peer_handle.await.unwrap();
}

#[tokio::test]
async fn slow_peer_keepalive_but_no_data() {
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
    .add_event(ScriptedEvent::Send(PeerMessage::KeepAlive))
    .add_event(ScriptedEvent::Send(PeerMessage::KeepAlive))
    .add_event(ScriptedEvent::Send(PeerMessage::KeepAlive))
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

fn block(piece: u32, offset: u32, length: u32, piece_length: u32) -> BlockSpec {
    BlockSpec::new(
        PieceIndex::new(piece),
        BlockOffset::new(offset),
        BlockLength::new(length).unwrap(),
        piece_length,
    )
    .unwrap()
}
