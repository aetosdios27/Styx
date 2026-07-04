use std::time::Duration;

use styx_runtime::{
    FailureScope, RetryClass, RuntimeConfig, RuntimeError, RuntimeLimits, SmokeConfig,
    SmokeRunConfig, SmokeStage, SmokeTarget,
};

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

#[test]
fn runtime_config_default_uses_bounded_phase_13_limits() {
    let config = RuntimeConfig::default();

    assert_eq!(config.connect_timeout, Duration::from_secs(10));
    assert_eq!(config.source_timeout, Duration::from_secs(15));
    assert_eq!(config.piece_timeout, Duration::from_secs(30));
    assert_eq!(config.snapshot_interval, Duration::from_secs(1));
    assert_eq!(config.limits.max_active_torrents, 8);
    assert_eq!(config.limits.max_peers_per_torrent, 30);
    assert_eq!(config.limits.max_sources_per_torrent, 64);
    assert_eq!(config.limits.max_web_seed_concurrency, 2);
    assert_eq!(config.limits.max_event_queue, 1024);
    assert_eq!(config.limits.source_retry_limit, 3);
    assert_eq!(config.peer.request_pipeline_depth, 5);
}

#[test]
fn runtime_config_rejects_zero_runtime_limit() {
    let config = RuntimeConfig {
        limits: RuntimeLimits {
            max_event_queue: 0,
            ..RuntimeLimits::default()
        },
        ..RuntimeConfig::default()
    };

    assert_eq!(
        config.validate().unwrap_err(),
        RuntimeError::InvalidConfig("max_event_queue must be greater than zero")
    );
}

#[test]
fn runtime_config_rejects_invalid_peer_manager_config() {
    let mut config = RuntimeConfig::default();
    config.peer.request_pipeline_depth = 0;

    assert_eq!(
        config.validate().unwrap_err(),
        RuntimeError::InvalidConfig("request_pipeline_depth must be greater than zero")
    );
}

#[test]
fn runtime_error_classifies_source_timeout_as_retryable_source_failure() {
    let err = RuntimeError::SourceFailed {
        source_id: "peer:127.0.0.1:6881".to_owned(),
        scope: FailureScope::SourceLocal,
        retry: RetryClass::Retryable,
        reason: "connect timeout".to_owned(),
    };

    assert_eq!(err.scope(), FailureScope::SourceLocal);
    assert_eq!(err.retry_class(), RetryClass::Retryable);
}
