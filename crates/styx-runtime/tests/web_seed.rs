use std::{collections::BTreeMap, ops::RangeInclusive};

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_disk::PieceIndex;
use styx_proto::{encode, BencodeValue};
use styx_runtime::{piece_byte_range, validate_web_seed_piece_bytes, RuntimeError, TorrentPlan};

#[test]
fn piece_byte_range_returns_first_middle_and_final_piece_ranges() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(
        &torrent,
        torrent_with_piece_lengths(&[b"abcd", b"efgh", b"ij"]),
    )
    .unwrap();
    let plan = TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();

    assert_eq!(
        piece_byte_range(&plan, PieceIndex::new(0)).unwrap(),
        RangeInclusive::new(0, 3)
    );
    assert_eq!(
        piece_byte_range(&plan, PieceIndex::new(1)).unwrap(),
        RangeInclusive::new(4, 7)
    );
    assert_eq!(
        piece_byte_range(&plan, PieceIndex::new(2)).unwrap(),
        RangeInclusive::new(8, 9)
    );
}

#[test]
fn validate_web_seed_piece_bytes_rejects_short_response() {
    let err =
        validate_web_seed_piece_bytes(PieceIndex::new(2), 2, Bytes::from_static(b"i")).unwrap_err();

    assert_eq!(
        err,
        RuntimeError::InvalidWebSeedLength {
            piece: 2,
            expected: 2,
            actual: 1,
        }
    );
}

#[test]
fn validate_web_seed_piece_bytes_accepts_exact_length_response() {
    let bytes =
        validate_web_seed_piece_bytes(PieceIndex::new(2), 2, Bytes::from_static(b"ij")).unwrap();

    assert_eq!(bytes, Bytes::from_static(b"ij"));
}

fn torrent_with_piece_lengths(chunks: &[&[u8]]) -> Vec<u8> {
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
    info.insert(b"piece length".to_vec(), BencodeValue::Integer(4));
    info.insert(
        b"length".to_vec(),
        BencodeValue::Integer(chunks.iter().map(|chunk| chunk.len() as i64).sum()),
    );
    let mut pieces = Vec::new();
    for chunk in chunks {
        pieces.extend_from_slice(&Sha1::digest(chunk));
    }
    info.insert(b"pieces".to_vec(), BencodeValue::Bytes(Bytes::from(pieces)));
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}
