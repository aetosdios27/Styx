use std::path::PathBuf;

use styx_runtime::{
    PersistentState, PersistentStore, PersistentTorrent, PersistentTorrentState, RuntimeError,
};

#[test]
fn persistent_state_round_trips_torrent_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let store = PersistentStore::open(temp.path()).unwrap();
    let source_path = PathBuf::from("/tmp/styx/sample.torrent");
    let destination = PathBuf::from("/tmp/styx/downloads");
    let state = PersistentState {
        schema_version: 1,
        torrents: vec![PersistentTorrent {
            source_path: source_path.clone(),
            destination: destination.clone(),
            state: PersistentTorrentState::Downloading,
            added_at_unix: 1_725_000_000,
            completed_at_unix: None,
        }],
    };

    store.save(&state).unwrap();

    let restored = store.load().unwrap();
    assert_eq!(restored, state);
}

#[test]
fn persistent_store_rejects_unknown_schema_version() {
    let temp = tempfile::tempdir().unwrap();
    let store = PersistentStore::open(temp.path()).unwrap();
    std::fs::write(
        temp.path().join("state.json"),
        r#"{"schema_version":999,"torrents":[]}"#,
    )
    .unwrap();

    let err = store.load().unwrap_err();

    assert_eq!(
        err,
        RuntimeError::Persistence("unsupported persistent state schema version")
    );
}

#[test]
fn persistent_store_rejects_corrupt_json_without_deleting_file() {
    let temp = tempfile::tempdir().unwrap();
    let store = PersistentStore::open(temp.path()).unwrap();
    let path = temp.path().join("state.json");
    std::fs::write(&path, b"{not-json").unwrap();

    let err = store.load().unwrap_err();

    assert_eq!(
        err,
        RuntimeError::Persistence("invalid persistent state json")
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"{not-json");
}
