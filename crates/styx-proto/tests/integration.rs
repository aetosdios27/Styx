use std::collections::BTreeMap;

use bytes::Bytes;
use styx_proto::bencode::{decode, encode, BencodeError, BencodeValue};
use styx_proto::peer::{
    decode_handshake, decode_message_frame, encode_handshake, encode_message, ExtensionBits,
    Handshake, PeerId, PeerMessage, PEER_HANDSHAKE_LEN,
};
use styx_proto::{decode_torrent, FileMode, InfoHashV1, TorrentFile};

#[test]
fn bencode_complex_nested_round_trip() {
    let mut inner = BTreeMap::new();
    inner.insert(b"key1".to_vec(), BencodeValue::Integer(42));
    inner.insert(
        b"key2".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"val")),
    );
    let mut outer = BTreeMap::new();
    outer.insert(
        b"list".to_vec(),
        BencodeValue::List(vec![
            BencodeValue::Integer(1),
            BencodeValue::Integer(2),
            BencodeValue::Integer(3),
        ]),
    );
    outer.insert(b"nested".to_vec(), BencodeValue::Dict(inner));
    outer.insert(b"flag".to_vec(), BencodeValue::Integer(0));
    let value = BencodeValue::Dict(outer);

    let encoded = encode(&value);
    let decoded = decode(&encoded).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn peer_wire_handshake_encodes_and_decodes() {
    let hs = Handshake {
        reserved: ExtensionBits::new([0, 0, 0, 0, 0, 0, 0, 1]),
        info_hash: InfoHashV1::new([0xab; 20]),
        peer_id: PeerId::new([0x42; 20]),
    };
    let encoded = encode_handshake(&hs);
    assert_eq!(encoded.len(), PEER_HANDSHAKE_LEN);

    let decoded = decode_handshake(&encoded).unwrap();
    assert_eq!(decoded, hs);
}

#[test]
fn peer_wire_interested_and_unchoke_round_trip() {
    for msg in &[PeerMessage::Interested, PeerMessage::Unchoke] {
        let encoded = encode_message(msg).unwrap();
        let decoded = decode_message_frame(&encoded).unwrap();
        assert_eq!(decoded, *msg);
    }
}

#[test]
fn peer_wire_request_piece_cancel_round_trip() {
    let messages = vec![
        PeerMessage::Request {
            index: 0,
            begin: 0,
            length: 16_384,
        },
        PeerMessage::Piece {
            index: 1,
            begin: 512,
            block: Bytes::from_static(&[0xAB; 1024]),
        },
        PeerMessage::Cancel {
            index: 2,
            begin: 0,
            length: 8192,
        },
    ];
    for msg in &messages {
        let encoded = encode_message(msg).unwrap();
        let decoded = decode_message_frame(&encoded).unwrap();
        assert_eq!(decoded, *msg);
    }
}

#[test]
fn metainfo_decode_v1_single_file_fixture() {
    let metainfo = decode_torrent(fixture(include_bytes!("fixtures/single_file.torrent"))).unwrap();
    assert_eq!(
        metainfo.announce.as_deref(),
        Some(b"http://tracker.example/announce".as_slice())
    );
    assert_eq!(metainfo.info.name.as_ref(), b"hello.txt");
    assert_eq!(metainfo.info.mode, FileMode::Single { length: 12_345 });
}

#[test]
fn metainfo_decode_v1_multi_file_fixture() {
    let metainfo = decode_torrent(fixture(include_bytes!("fixtures/multi_file.torrent"))).unwrap();
    assert_eq!(metainfo.info.name.as_ref(), b"album");
    assert_eq!(
        metainfo.info.mode,
        FileMode::Multi {
            files: vec![
                TorrentFile {
                    length: 3,
                    path: vec![Bytes::from_static(b"one.txt")]
                },
                TorrentFile {
                    length: 4,
                    path: vec![Bytes::from_static(b"dir"), Bytes::from_static(b"two.bin")]
                },
            ]
        }
    );
}

#[test]
fn metainfo_decode_hybrid_torrent() {
    let input = make_hybrid_bytes();
    let metainfo = decode_torrent(&input).unwrap();
    assert!(metainfo.info.pieces.is_some());
    assert!(metainfo.info_hash_v2.is_some());
    assert_eq!(metainfo.info.meta_version, Some(2));
}

#[test]
fn bencode_depth_limit_is_enforced() {
    let depth = 130;
    let mut input = vec![b'l'; depth + 2];
    input.extend(std::iter::repeat_n(b'e', depth + 2));
    let err = decode(&input).unwrap_err();
    assert_eq!(
        err,
        BencodeError::DepthLimitExceeded {
            offset: 129,
            limit: 128,
        }
    );
}

fn fixture(input: &'static [u8]) -> &'static [u8] {
    input.strip_suffix(b"\n").unwrap_or(input)
}

fn make_hybrid_bytes() -> Vec<u8> {
    let v1_pieces: Vec<u8> = (0..20).map(|i| i as u8).collect();
    let pieces_root: Vec<u8> = (0..32).map(|i| i as u8).collect();

    let mut file_entry = BTreeMap::new();
    file_entry.insert(b"length".to_vec(), BencodeValue::Integer(1024));
    file_entry.insert(
        b"pieces root".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(&pieces_root)),
    );
    let mut file_marker = BTreeMap::new();
    file_marker.insert(Vec::new(), BencodeValue::Dict(file_entry));
    let mut file_tree_root = BTreeMap::new();
    file_tree_root.insert(b"test".to_vec(), BencodeValue::Dict(file_marker));

    let mut info = BTreeMap::new();
    info.insert(b"file tree".to_vec(), BencodeValue::Dict(file_tree_root));
    info.insert(b"length".to_vec(), BencodeValue::Integer(1024));
    info.insert(b"meta version".to_vec(), BencodeValue::Integer(2));
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"test")),
    );
    info.insert(b"piece length".to_vec(), BencodeValue::Integer(16384));
    info.insert(
        b"pieces".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(&v1_pieces)),
    );

    let mut top = BTreeMap::new();
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}
