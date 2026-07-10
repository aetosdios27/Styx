use styx_app::{commands::CommandResponse, ControlCommand, TorrentRuntime};
use styx_runtime::{AppRuntime, RuntimeConfig};

#[test]
fn app_runtime_without_session_keeps_synchronous_status_contract() {
    let mut runtime = AppRuntime::new_with_config(RuntimeConfig::default()).unwrap();

    let response = runtime.apply(ControlCommand::Status).unwrap();

    let CommandResponse::Status { snapshot } = response else {
        panic!("expected status response");
    };
    assert!(snapshot.torrents.is_empty());
    assert_eq!(snapshot.totals.torrent_count, 0);
}
