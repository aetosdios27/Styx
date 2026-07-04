use styx_proto::InfoHashV1;
use styx_runtime::{IntentState, RollbackRecord, SettingsPatch, StageIntent, TorrentId};

fn tid(byte: u8) -> TorrentId {
    TorrentId::new(InfoHashV1::new([byte; 20]))
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
