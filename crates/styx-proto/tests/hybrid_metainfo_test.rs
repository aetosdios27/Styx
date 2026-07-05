use std::collections::BTreeMap;

use styx_proto::bencode::{encode, BencodeValue};
use styx_proto::{decode_torrent, is_hybrid};

fn make_hybrid_torrent_bytes() -> Vec<u8> {
    let pieces_root: Vec<u8> = (0..32).map(|i| i as u8).collect();
    let v1_pieces: Vec<u8> = (0..20).map(|i| i as u8).collect();

    // Build the file tree: { "test": { "": { "length": 1024, "pieces root": <32 bytes> } } }
    let mut file_entry = BTreeMap::new();
    file_entry.insert(
        b"length".to_vec(),
        BencodeValue::Integer(1024),
    );
    file_entry.insert(
        b"pieces root".to_vec(),
        BencodeValue::Bytes(pieces_root.clone().into()),
    );

    let mut file_marker = BTreeMap::new();
    file_marker.insert(
        Vec::new(), // empty key marks as file
        BencodeValue::Dict(file_entry),
    );

    let mut file_tree_root = BTreeMap::new();
    file_tree_root.insert(
        b"test".to_vec(),
        BencodeValue::Dict(file_marker),
    );

    // Build the info dict with both v1 and v2 fields
    let mut info = BTreeMap::new();
    info.insert(
        b"file tree".to_vec(),
        BencodeValue::Dict(file_tree_root),
    );
    info.insert(
        b"length".to_vec(),
        BencodeValue::Integer(1024),
    );
    info.insert(
        b"meta version".to_vec(),
        BencodeValue::Integer(2),
    );
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(b"test".as_slice().into()),
    );
    info.insert(
        b"piece length".to_vec(),
        BencodeValue::Integer(16384),
    );
    info.insert(
        b"pieces".to_vec(),
        BencodeValue::Bytes(v1_pieces.clone().into()),
    );

    let mut top = BTreeMap::new();
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));

    encode(&BencodeValue::Dict(top))
}

#[test]
fn detect_hybrid_torrent() {
    let hybrid_bytes = make_hybrid_torrent_bytes();
    let parsed = decode_torrent(&hybrid_bytes).unwrap();
    assert!(is_hybrid(&parsed));
    assert!(parsed.info_hash_v2.is_some());
    assert!(parsed.info.pieces.is_some());
}
