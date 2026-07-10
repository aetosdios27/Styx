use bytes::Bytes;
use std::collections::BTreeMap;
use styx_proto::{decode_torrent, encode, BencodeValue};
use styx_runtime::{
    DiscoveryPolicy, RuntimeCommand, RuntimeConfig, RuntimeEngine, TorrentCommand, TorrentPlan,
};

#[test]
fn private_torrent_disables_all_decentralized_discovery() {
    let metainfo = decode_torrent(&private_torrent_bytes()).unwrap();
    let policy = DiscoveryPolicy::from_metainfo(&metainfo);

    assert!(!policy.dht_allowed());
    assert!(!policy.pex_allowed());
    assert!(!policy.lsd_allowed());
    assert!(!policy.port_message_allowed());
}

#[test]
fn decentralized_plan_preserves_private_flag_for_runtime_enforcement() {
    let temp = tempfile::tempdir().unwrap();
    let metainfo = decode_torrent(&private_torrent_bytes()).unwrap();

    let plan = TorrentPlan::from_metainfo_decentralized(metainfo, temp.path()).unwrap();

    assert!(plan.is_private());
}

#[test]
fn private_torrent_is_never_a_dht_announce_target() {
    let temp = tempfile::tempdir().unwrap();
    let metainfo = decode_torrent(&private_torrent_bytes()).unwrap();
    let plan = TorrentPlan::from_metainfo_decentralized(metainfo, temp.path()).unwrap();
    let id = plan.id;
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();

    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();
    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))
        .unwrap();

    assert!(engine.dht_announce_targets().is_empty());
    assert_eq!(engine.dht_announce_target(id), None);
    assert!(engine.lsd_announce_targets().is_empty());
}

fn private_torrent_bytes() -> Vec<u8> {
    let mut info = BTreeMap::new();
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"private.bin")),
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
    info.insert(b"private".to_vec(), BencodeValue::Integer(1));
    let mut root = BTreeMap::new();
    root.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(root))
}
