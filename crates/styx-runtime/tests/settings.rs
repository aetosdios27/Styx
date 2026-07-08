use std::collections::BTreeMap;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_proto::{encode, BencodeValue};
use styx_runtime::{
    RuntimeCommand, RuntimeConfig, RuntimeEngine, SeedPolicy, SettingsPatch, StageIntent,
    TorrentPlan, TorrentStatus,
};

#[test]
fn default_runtime_config_seeds_after_completion() {
    assert!(RuntimeConfig::default().seed_policy.seed_after_complete);
}

#[test]
fn settings_patch_can_disable_seed_after_completion() {
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let patch = SettingsPatch {
        seed_policy: Some(SeedPolicy {
            seed_after_complete: false,
        }),
        ..SettingsPatch::default()
    };
    let intent = StageIntent::Settings { patch };

    intent.validate(&engine).unwrap();
    intent.execute(&mut engine).unwrap();

    assert!(!engine.config().seed_policy.seed_after_complete);
}

#[tokio::test]
async fn seed_after_complete_false_leaves_torrent_complete_not_seeding() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let plan = TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let id = plan.id;
    let mut engine = RuntimeEngine::new(RuntimeConfig {
        seed_policy: SeedPolicy {
            seed_after_complete: false,
        },
        ..RuntimeConfig::default()
    })
    .unwrap();

    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();
    engine
        .complete_from_piece_bytes(id, vec![Bytes::from_static(b"abcd")])
        .await
        .unwrap();

    assert_eq!(
        engine.snapshot().torrents[0].status,
        TorrentStatus::Complete
    );
}

fn torrent_bytes() -> Vec<u8> {
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
    info.insert(b"length".to_vec(), BencodeValue::Integer(4));
    info.insert(
        b"pieces".to_vec(),
        BencodeValue::Bytes(Bytes::copy_from_slice(&Sha1::digest(b"abcd"))),
    );
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}
