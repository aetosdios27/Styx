use std::collections::BTreeMap;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_proto::{encode, BencodeValue, InfoHashV1};
use styx_runtime::{
    IntentState, RollbackRecord, RuntimeCommand, RuntimeConfig, RuntimeEngine, RuntimeError,
    SettingsPatch, StageIntent, TorrentId, TorrentPlan,
};

fn tid(byte: u8) -> TorrentId {
    TorrentId::new(InfoHashV1::new([byte; 20]))
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

fn plan() -> (tempfile::TempDir, TorrentPlan) {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let plan = TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    (temp, plan)
}

#[test]
fn intent_stage_defaults_to_declared() {
    let intent = StageIntent::Pause { id: tid(1) };
    assert_eq!(intent.state(), IntentState::Declared);
}

#[test]
fn settings_patch_default_fields_are_none() {
    let patch = SettingsPatch::default();
    assert!(patch.listen_port.is_none());
    assert!(patch.limits.is_none());
}

#[test]
fn rollback_record_add_stores_id() {
    let id = tid(42);
    let record = RollbackRecord::AddRollback { id };
    assert!(matches!(record, RollbackRecord::AddRollback { id } if id == tid(42)));
}

#[test]
fn add_intent_rejects_duplicate_torrent_id() {
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let (_tmp, plan) = plan();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan.clone())))
        .unwrap();

    let intent = StageIntent::Add {
        plan: Box::new(plan),
    };
    let result = intent.validate(&engine);
    assert!(matches!(
        result,
        Err(RuntimeError::InvalidConfig("torrent already exists"))
    ));
}

#[test]
fn remove_intent_rejects_unknown_torrent() {
    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let (_tmp, plan) = plan();
    let intent = StageIntent::Remove {
        id: plan.id,
        delete_data: false,
    };
    let result = intent.validate(&engine);
    assert!(matches!(
        result,
        Err(RuntimeError::InvalidConfig("unknown torrent"))
    ));
}

#[test]
fn pause_intent_rejects_unknown_torrent() {
    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let (_tmp, plan) = plan();
    let intent = StageIntent::Pause { id: plan.id };
    let result = intent.validate(&engine);
    assert!(matches!(
        result,
        Err(RuntimeError::InvalidConfig("unknown torrent"))
    ));
}

#[test]
fn resume_intent_rejects_unknown_torrent() {
    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let (_tmp, plan) = plan();
    let intent = StageIntent::Resume { id: plan.id };
    let result = intent.validate(&engine);
    assert!(matches!(
        result,
        Err(RuntimeError::InvalidConfig("unknown torrent"))
    ));
}

#[test]
fn settings_intent_rejects_zero_listen_port() {
    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let patch = SettingsPatch {
        listen_port: Some(0),
        limits: None,
    };
    let intent = StageIntent::Settings { patch };
    let result = intent.validate(&engine);
    assert!(matches!(
        result,
        Err(RuntimeError::InvalidConfig("listen port must be non-zero"))
    ));
}
