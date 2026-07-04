use std::collections::BTreeMap;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_proto::{encode, BencodeValue, InfoHashV1};
use styx_runtime::{
    IntentState, RollbackRecord, RuntimeCommand, RuntimeConfig, RuntimeEngine, RuntimeError,
    RuntimeEvent, RuntimeLimits, SettingsPatch, StageIntent, TorrentCommand, TorrentId,
    TorrentPlan, TorrentStatus,
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

fn engine_with_one_torrent() -> (tempfile::TempDir, RuntimeEngine, TorrentId) {
    let (tmp, plan) = plan();
    let id = plan.id;
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();
    (tmp, engine, id)
}

#[test]
fn add_intent_execute_inserts_task() {
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let (_tmp, plan) = plan();
    let id = plan.id;
    let intent = StageIntent::Add {
        plan: Box::new(plan),
    };

    let record = intent.execute(&mut engine).unwrap();
    assert!(engine.has_torrent(id));
    assert!(matches!(record, Some(RollbackRecord::AddRollback { .. })));
}

#[test]
fn add_intent_rollback_removes_task() {
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let (_tmp, plan) = plan();
    let id = plan.id;
    let intent = StageIntent::Add {
        plan: Box::new(plan),
    };

    let record = intent.execute(&mut engine).unwrap();
    assert!(engine.has_torrent(id));
    engine
        .rollback(record.expect("Add execute returns AddRollback"))
        .unwrap();
    assert!(!engine.has_torrent(id));
}

#[test]
fn remove_intent_execute_removes_task() {
    let (_tmp, mut engine, id) = engine_with_one_torrent();
    let intent = StageIntent::Remove {
        id,
        delete_data: false,
    };

    let record = intent.execute(&mut engine).unwrap();
    assert!(!engine.has_torrent(id));
    assert!(matches!(
        record,
        Some(RollbackRecord::RemoveRollback { .. })
    ));
}

#[test]
fn remove_intent_rollback_restores_task() {
    let (_tmp, mut engine, id) = engine_with_one_torrent();
    let intent = StageIntent::Remove {
        id,
        delete_data: false,
    };

    let record = intent.execute(&mut engine).unwrap();
    assert!(!engine.has_torrent(id));
    engine
        .rollback(record.expect("Remove execute returns RemoveRollback"))
        .unwrap();
    assert!(engine.has_torrent(id));
}

#[test]
fn pause_intent_execute_pauses_task() {
    let (_tmp, mut engine, id) = engine_with_one_torrent();
    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))
        .unwrap();

    let intent = StageIntent::Pause { id };
    let record = intent.execute(&mut engine).unwrap();
    assert!(record.is_none());
    let snap = engine.snapshot();
    assert_eq!(snap.torrents[0].status, TorrentStatus::Paused);
}

#[test]
fn resume_intent_execute_resumes_task() {
    let (_tmp, mut engine, id) = engine_with_one_torrent();
    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))
        .unwrap();
    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Pause))
        .unwrap();

    let intent = StageIntent::Resume { id };
    let record = intent.execute(&mut engine).unwrap();
    assert!(record.is_none());
    let snap = engine.snapshot();
    assert_eq!(snap.torrents[0].status, TorrentStatus::Downloading);
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

#[test]
fn settings_intent_apply_patch_updates_config() {
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let new_limits = RuntimeLimits {
        max_active_torrents: 16,
        ..engine.config().limits
    };
    let patch = SettingsPatch {
        limits: Some(new_limits),
        listen_port: None,
    };
    let intent = StageIntent::Settings { patch };

    intent.validate(&engine).unwrap();
    let record = intent.execute(&mut engine).unwrap();
    assert_eq!(engine.config().limits.max_active_torrents, 16);
    assert!(matches!(
        record,
        Some(RollbackRecord::SettingsRollback { .. })
    ));
}

#[test]
fn settings_intent_rollback_restores_previous_config() {
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let old_limits = engine.config().limits;
    let patch = SettingsPatch {
        limits: Some(RuntimeLimits {
            max_active_torrents: 16,
            ..old_limits
        }),
        listen_port: None,
    };
    let intent = StageIntent::Settings { patch };

    let record = intent.execute(&mut engine).unwrap();
    engine.rollback(record.unwrap()).unwrap();
    assert_eq!(engine.config().limits, old_limits);
}

#[test]
fn intent_declare_emits_declared_event() {
    let (_tmp, plan) = plan();
    let intent = StageIntent::Add {
        plan: Box::new(plan),
    };

    let events = intent.declare();
    assert!(events
        .iter()
        .any(|e| matches!(e, RuntimeEvent::IntentDeclared { .. })));
}

#[test]
fn successful_execution_emits_success_event() {
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let (_tmp, plan) = plan();
    let intent = StageIntent::Add {
        plan: Box::new(plan),
    };

    let events = intent.run(&mut engine).unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind()).collect();
    assert!(kinds.contains(&"execution_succeeded"));
}

#[test]
fn failed_validation_emits_error() {
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let patch = SettingsPatch {
        listen_port: Some(0),
        limits: None,
    };
    let intent = StageIntent::Settings { patch };

    let result = intent.run(&mut engine);
    assert!(result.is_err());
}

#[test]
fn settings_intent_none_fields_leave_config_unchanged() {
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let original = engine.config().limits;
    let patch = SettingsPatch {
        limits: None,
        listen_port: None,
    };
    let intent = StageIntent::Settings { patch };

    intent.validate(&engine).unwrap();
    intent.execute(&mut engine).unwrap();
    assert_eq!(engine.config().limits, original);
}
