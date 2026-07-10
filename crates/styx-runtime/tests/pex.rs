use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};

use bytes::Bytes;
use styx_proto::{decode_torrent, encode, BencodeValue};
use styx_runtime::{RuntimeConfig, TorrentPlan, TorrentTask};

#[test]
fn pex_source_candidates_are_capped_deduplicated_and_filtered() {
    let temp = tempfile::tempdir().unwrap();
    let metainfo = decode_torrent(&torrent_bytes(false)).unwrap();
    let plan = TorrentPlan::from_metainfo_decentralized(metainfo, temp.path()).unwrap();
    let mut task = TorrentTask::new_with_peers(plan, RuntimeConfig::default()).unwrap();
    let mut peers: Vec<_> = (1..=60)
        .map(|port| SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), port)))
        .collect();
    peers.push(peers[0]);
    peers.push(SocketAddr::from((Ipv4Addr::LOCALHOST, 6881)));
    peers.push(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 6881)));

    let added = task.ingest_pex_peers(peers);

    assert_eq!(added, 50);
}

#[test]
fn private_torrent_ignores_pex_messages() {
    let temp = tempfile::tempdir().unwrap();
    let metainfo = decode_torrent(&torrent_bytes(true)).unwrap();
    let plan = TorrentPlan::from_metainfo_decentralized(metainfo, temp.path()).unwrap();
    let mut task = TorrentTask::new_with_peers(plan, RuntimeConfig::default()).unwrap();

    let added = task.ingest_pex_peers([SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 6881))]);

    assert_eq!(added, 0);
}

fn torrent_bytes(private: bool) -> Vec<u8> {
    let mut info = BTreeMap::new();
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"pex.bin")),
    );
    info.insert(b"piece length".to_vec(), BencodeValue::Integer(4));
    info.insert(b"length".to_vec(), BencodeValue::Integer(4));
    info.insert(
        b"pieces".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(&[
            0x81, 0xfe, 0x8b, 0xfe, 0x87, 0x57, 0x6c, 0x3e, 0xcb, 0x22, 0x42, 0x6f, 0x8e, 0x57,
            0x84, 0x73, 0x82, 0x91, 0x7a, 0xcf,
        ])),
    );
    if private {
        info.insert(b"private".to_vec(), BencodeValue::Integer(1));
    }
    let mut root = BTreeMap::new();
    root.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(root))
}
