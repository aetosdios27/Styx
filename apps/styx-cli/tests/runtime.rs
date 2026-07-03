use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use styx_cli::{
    commands::{CommandResponse, ControlCommand},
    format::InfoHashHex,
    model::TorrentStatus,
    runtime::{MemoryRuntime, TorrentRuntime},
};

fn fixture(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("styx-cli-{name}-{}-{nanos}", std::process::id()));
    fs::write(
        &path,
        b"d8:announce31:http://tracker.example/announce4:infod6:lengthi12345e4:name9:hello.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee",
    )
    .unwrap();
    path
}

#[test]
fn memory_runtime_add_parses_torrent_metadata() {
    let mut runtime = MemoryRuntime::default();

    let response = runtime
        .apply(ControlCommand::Add {
            source: fixture("single_file.torrent"),
            destination: None,
        })
        .unwrap();

    let CommandResponse::TorrentAdded { name, .. } = response else {
        panic!("expected torrent_added response");
    };
    assert_eq!(name, "hello.txt");
}

#[test]
fn memory_runtime_rejects_duplicate_info_hash() {
    let mut runtime = MemoryRuntime::default();
    let command = ControlCommand::Add {
        source: fixture("single_file.torrent"),
        destination: None,
    };
    runtime.apply(command.clone()).unwrap();

    let err = runtime.apply(command).unwrap_err();

    assert!(err.to_string().contains("already exists"));
}

#[test]
fn memory_runtime_pause_and_resume_update_status() {
    let mut runtime = MemoryRuntime::default();
    let CommandResponse::TorrentAdded { info_hash, .. } = runtime
        .apply(ControlCommand::Add {
            source: fixture("single_file.torrent"),
            destination: None,
        })
        .unwrap()
    else {
        panic!("expected torrent_added response");
    };

    runtime.apply(ControlCommand::Pause { info_hash }).unwrap();
    assert_eq!(runtime.snapshot().torrents[0].status, TorrentStatus::Paused);

    runtime.apply(ControlCommand::Resume { info_hash }).unwrap();
    assert_eq!(
        runtime.snapshot().torrents[0].status,
        TorrentStatus::Checking
    );
}

#[test]
fn memory_runtime_unknown_hash_returns_error() {
    let mut runtime = MemoryRuntime::default();

    let err = runtime
        .apply(ControlCommand::Remove {
            info_hash: InfoHashHex::repeat(0xaa),
        })
        .unwrap_err();

    assert!(err.to_string().contains("unknown torrent"));
}

#[test]
fn control_command_round_trips_as_tagged_json() {
    let command = ControlCommand::Status;

    let json = serde_json::to_string(&command).unwrap();
    let decoded: ControlCommand = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded, command);
}
