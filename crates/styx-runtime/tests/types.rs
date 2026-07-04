use std::time::Duration;

use styx_runtime::{RuntimeError, SmokeConfig, SmokeRunConfig, SmokeStage, SmokeTarget};

#[test]
fn smoke_config_default_uses_bounded_network_timeouts() {
    let config = SmokeConfig::default();

    assert_eq!(config.connect_timeout, Duration::from_secs(10));
    assert_eq!(config.peer_message_timeout, Duration::from_secs(15));
    assert_eq!(config.max_tracker_response_bytes, 512 * 1024);
    assert_eq!(config.numwant, 30);
}

#[test]
fn smoke_config_rejects_zero_timeouts() {
    let config = SmokeConfig {
        connect_timeout: Duration::ZERO,
        ..SmokeConfig::default()
    };

    assert_eq!(
        config.validate().unwrap_err(),
        RuntimeError::InvalidConfig("connect_timeout must be greater than zero")
    );
}

#[test]
fn smoke_target_defaults_to_first_piece() {
    assert_eq!(SmokeTarget::default(), SmokeTarget::FirstPiece);
}

#[test]
fn smoke_stage_names_are_stable_for_logs() {
    assert_eq!(SmokeStage::Announcing.as_str(), "announcing");
    assert_eq!(SmokeStage::Verifying.as_str(), "verifying");
}

#[test]
fn smoke_run_config_rejects_zero_listen_port() {
    let config = SmokeRunConfig {
        torrent_path: "sample.torrent".into(),
        destination: "downloads".into(),
        listen_port: 0,
        ..SmokeRunConfig::default_for_paths("sample.torrent", "downloads")
    };

    assert_eq!(
        config.validate().unwrap_err(),
        RuntimeError::InvalidConfig("listen_port must be greater than zero")
    );
}
