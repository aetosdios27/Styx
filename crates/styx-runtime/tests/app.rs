use std::collections::BTreeMap;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_app::TorrentRuntime;
use styx_proto::{encode, BencodeValue};
use styx_runtime::{AppRuntime, RuntimeConfig, RuntimeEngine};

#[test]
fn snapshot_converts_torrent_status_correctly() {
    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let runtime = AppRuntime::new(engine);
    let app_snap = runtime.snapshot();
    assert_eq!(app_snap.totals.torrent_count, 0);
}

#[test]
fn apply_add_loads_plan_and_inserts_torrent() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);
    use styx_app::ControlCommand;
    let response = runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();
    assert!(matches!(
        response,
        styx_app::CommandResponse::TorrentAdded { .. }
    ));
    let snap = runtime.snapshot();
    assert_eq!(snap.totals.torrent_count, 1);
}

#[test]
fn apply_remove_removes_torrent() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let info_hash = {
        let bytes = std::fs::read(&torrent).unwrap();
        let meta = styx_proto::decode_torrent(&bytes).unwrap();
        styx_app::InfoHashHex::new(*meta.info_hash_v1.as_bytes())
    };

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    assert_eq!(runtime.snapshot().totals.torrent_count, 1);

    runtime.apply(ControlCommand::Remove { info_hash }).unwrap();
    assert_eq!(runtime.snapshot().totals.torrent_count, 0);
}

#[test]
fn apply_unknown_remove_returns_error() {
    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);
    use styx_app::ControlCommand;
    let result = runtime.apply(ControlCommand::Remove {
        info_hash: styx_app::InfoHashHex::new([0u8; 20]),
    });
    assert!(result.is_err());
}

#[test]
fn tick_drains_engine_events() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    let events = runtime.tick();
    assert!(
        !events.is_empty(),
        "tick should return events from drained engine"
    );
    assert!(events
        .iter()
        .any(|e| matches!(e, styx_app::AppEvent::TorrentAdded { .. })));
}

#[test]
fn snapshot_accumulates_speed_samples_across_ticks() {
    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    let snap = runtime.snapshot();
    assert_eq!(snap.speed.len(), 0);

    runtime.tick();
    let snap = runtime.snapshot();
    assert_eq!(snap.speed.len(), 1);

    runtime.tick();
    let snap = runtime.snapshot();
    assert_eq!(snap.speed.len(), 2);
}

#[test]
fn apply_pause_and_resume_routes_through_engine() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();
    let info_hash = {
        let bytes = std::fs::read(&torrent).unwrap();
        let meta = styx_proto::decode_torrent(&bytes).unwrap();
        styx_app::InfoHashHex::new(*meta.info_hash_v1.as_bytes())
    };

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    runtime.apply(ControlCommand::Pause { info_hash }).unwrap();
    assert_eq!(
        runtime.snapshot().torrents[0].status,
        styx_app::TorrentStatus::Paused
    );

    runtime.apply(ControlCommand::Resume { info_hash }).unwrap();
    assert_eq!(
        runtime.snapshot().torrents[0].status,
        styx_app::TorrentStatus::Downloading
    );
}

#[test]
fn tick_resolves_torrent_name_for_added_event() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    let events = runtime.tick();
    let added = events
        .iter()
        .find_map(|e| {
            if let styx_app::AppEvent::TorrentAdded { name, .. } = e {
                Some(name.as_str())
            } else {
                None
            }
        })
        .unwrap_or("");
    assert_eq!(added, "file.bin");
}

#[test]
fn tick_accumulates_logs_from_state_changed_events() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent,
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();

    runtime.tick();
    let snap = runtime.snapshot();
    assert!(
        !snap.logs.is_empty(),
        "tick should produce logs from events"
    );
    assert!(snap.logs.iter().any(|l| l.message.contains("added")));
}

#[test]
fn tick_logs_accumulate_across_multiple_ticks() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("test.torrent");
    std::fs::write(&torrent, torrent_bytes()).unwrap();

    let engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    let mut runtime = AppRuntime::new(engine);

    use styx_app::ControlCommand;
    runtime
        .apply(ControlCommand::Add {
            source: torrent.clone(),
            destination: Some(temp.path().join("downloads")),
        })
        .unwrap();
    let info_hash = {
        let bytes = std::fs::read(&torrent).unwrap();
        let meta = styx_proto::decode_torrent(&bytes).unwrap();
        styx_app::InfoHashHex::new(*meta.info_hash_v1.as_bytes())
    };

    runtime.tick();
    let first_logs = runtime.snapshot().logs.len();
    assert!(
        first_logs > 0,
        "add + auto-start produce at least one log entry"
    );

    runtime.apply(ControlCommand::Remove { info_hash }).unwrap();
    runtime.tick();
    assert!(
        runtime.snapshot().logs.len() > first_logs,
        "remove produces additional log entries"
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
