use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use styx_app::{CommandResponse, TorrentStatus};
use styx_gui::{
    commands::{add_torrent, get_snapshot, pause_torrent, resume_torrent},
    state::GuiState,
};

fn fixture(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("styx-gui-{name}-{}-{nanos}", std::process::id()));
    fs::write(
        &path,
        b"d8:announce31:http://tracker.example/announce4:infod6:lengthi12345e4:name9:hello.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee",
    )
    .unwrap();
    path
}

#[tokio::test]
async fn get_snapshot_returns_empty_runtime_state() {
    let state = GuiState::default();

    let snapshot = get_snapshot(&state).await.unwrap();

    assert!(snapshot.torrents.is_empty());
}

#[tokio::test]
async fn add_torrent_uses_shared_runtime_contract() {
    let state = GuiState::default();

    let response = add_torrent(&state, fixture("single_file.torrent"), None)
        .await
        .unwrap();

    let CommandResponse::TorrentAdded { name, .. } = response else {
        panic!("expected torrent_added response");
    };
    assert_eq!(name, "hello.txt");
}

#[tokio::test]
async fn pause_and_resume_update_snapshot_status() {
    let state = GuiState::default();
    let CommandResponse::TorrentAdded { info_hash, .. } =
        add_torrent(&state, fixture("single_file.torrent"), None)
            .await
            .unwrap()
    else {
        panic!("expected torrent_added response");
    };

    pause_torrent(&state, info_hash).await.unwrap();
    assert_eq!(
        get_snapshot(&state).await.unwrap().torrents[0].status,
        TorrentStatus::Paused
    );

    resume_torrent(&state, info_hash).await.unwrap();
    assert_eq!(
        get_snapshot(&state).await.unwrap().torrents[0].status,
        TorrentStatus::Checking
    );
}
