use styx_cli::{
    events::AppEvent,
    format::InfoHashHex,
    model::{AppSnapshot, LogLevel, TorrentRow, TorrentStatus},
};

#[test]
fn snapshot_event_serializes_with_stable_type_tag() {
    let snapshot = AppSnapshot {
        torrents: vec![TorrentRow {
            info_hash: InfoHashHex::repeat(0x11),
            name: "debian.iso".to_owned(),
            status: TorrentStatus::Paused,
            size_bytes: 1024,
            progress: 0.5,
            down_rate: 0,
            up_rate: 0,
            peers: 0,
            seeds: 0,
        }],
        ..AppSnapshot::default()
    };

    let value = serde_json::to_value(AppEvent::Snapshot { snapshot }).unwrap();

    assert_eq!(value["type"], "snapshot");
}

#[test]
fn command_failed_event_serializes_error_text() {
    let value = serde_json::to_value(AppEvent::CommandFailed {
        command: "add".to_owned(),
        error: "invalid torrent".to_owned(),
    })
    .unwrap();

    assert_eq!(value["error"], "invalid torrent");
}

#[test]
fn log_level_serializes_as_lowercase() {
    let value = serde_json::to_value(LogLevel::Warn).unwrap();

    assert_eq!(value, "warn");
}
