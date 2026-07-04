use std::collections::BTreeMap;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_disk::{BlockLength, BlockOffset, BlockSpec, PieceIndex};
use styx_proto::{encode, BencodeValue};
use styx_runtime::{PeerSessionDriver, SessionFailure, SessionOutcome, TorrentPlan};

#[tokio::test]
async fn peer_session_driver_verifies_piece_through_disk_manager() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes(b"abcdefgh")).unwrap();
    let plan = TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let mut driver = PeerSessionDriver::new(plan.disk_plan.clone());

    let outcome = driver
        .accept_piece_blocks(
            PieceIndex::new(0),
            vec![
                (block(0, 4, 4, 8), Bytes::from_static(b"efgh")),
                (block(0, 0, 4, 8), Bytes::from_static(b"abcd")),
            ],
        )
        .await
        .unwrap();

    assert_eq!(
        outcome,
        SessionOutcome::PieceVerified {
            piece: PieceIndex::new(0),
            bytes: 8
        }
    );
}

#[tokio::test]
async fn peer_session_driver_quarantines_corrupt_piece_payload() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes(b"abcdefgh")).unwrap();
    let plan = TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let mut driver = PeerSessionDriver::new(plan.disk_plan);

    let err = driver
        .accept_piece_blocks(
            PieceIndex::new(0),
            vec![
                (block(0, 0, 4, 8), Bytes::from_static(b"xxxx")),
                (block(0, 4, 4, 8), Bytes::from_static(b"yyyy")),
            ],
        )
        .await
        .unwrap_err();

    assert_eq!(
        err,
        SessionFailure::CorruptPiece {
            piece: PieceIndex::new(0)
        }
    );
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

fn torrent_bytes(piece: &[u8]) -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"url-list".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"https://mirror.test/")),
    );
    let mut info = BTreeMap::new();
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"file.bin")),
    );
    info.insert(
        b"piece length".to_vec(),
        BencodeValue::Integer(piece.len() as i64),
    );
    info.insert(
        b"length".to_vec(),
        BencodeValue::Integer(piece.len() as i64),
    );
    info.insert(
        b"pieces".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(&Sha1::digest(piece))),
    );
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}
