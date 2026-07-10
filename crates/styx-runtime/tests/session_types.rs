use std::time::Duration;

use styx_runtime::{
    FailureReasonCode, PersistenceOutcome, RuntimeConfig, RuntimeError, SessionRuntimeConfig,
    ShutdownMode, ShutdownReport,
};

#[test]
fn session_config_rejects_zero_capacities_and_deadlines() {
    for (config, expected) in [
        (
            SessionRuntimeConfig {
                command_capacity: 0,
                ..SessionRuntimeConfig::default()
            },
            "session command capacity must be greater than zero",
        ),
        (
            SessionRuntimeConfig {
                event_capacity: 0,
                ..SessionRuntimeConfig::default()
            },
            "session event capacity must be greater than zero",
        ),
        (
            SessionRuntimeConfig {
                shutdown_timeout: Duration::ZERO,
                ..SessionRuntimeConfig::default()
            },
            "session shutdown timeout must be greater than zero",
        ),
        (
            SessionRuntimeConfig {
                forced_shutdown_timeout: Duration::ZERO,
                ..SessionRuntimeConfig::default()
            },
            "session forced shutdown timeout must be greater than zero",
        ),
    ] {
        assert_eq!(
            config.validate().unwrap_err(),
            RuntimeError::InvalidConfig(expected)
        );
    }
}

#[test]
fn runtime_config_propagates_session_validation_error() {
    let config = RuntimeConfig {
        session: SessionRuntimeConfig {
            event_capacity: 0,
            ..SessionRuntimeConfig::default()
        },
        ..RuntimeConfig::default()
    };

    assert_eq!(
        config.validate().unwrap_err(),
        RuntimeError::InvalidConfig("session event capacity must be greater than zero")
    );
}

#[test]
fn shutdown_report_contains_only_task_kinds_counts_and_reason_codes() {
    let report = ShutdownReport::new(ShutdownMode::Clean, Duration::from_millis(10));
    let debug = format!("{report:?}");

    assert!(!debug.contains("127.0.0.1"));
    assert!(!debug.contains("magnet:"));
    assert!(!debug.contains("tracker"));
    assert_eq!(report.persistence, PersistenceOutcome::NotAttempted);
}

#[test]
fn persistence_failure_is_represented_only_by_stable_reason_code() {
    let mut report = ShutdownReport::new(ShutdownMode::Forced, Duration::from_secs(1));
    report.persistence = PersistenceOutcome::Failed(FailureReasonCode::PersistenceFailed);

    assert_eq!(
        report.persistence,
        PersistenceOutcome::Failed(FailureReasonCode::PersistenceFailed)
    );
}

#[test]
fn default_session_config_has_explicit_bounded_values() {
    let config = SessionRuntimeConfig::default();

    assert_eq!(config.command_capacity, 256);
    assert_eq!(config.event_capacity, 1024);
    assert_eq!(config.shutdown_timeout, Duration::from_secs(10));
    assert_eq!(config.forced_shutdown_timeout, Duration::from_secs(2));
    assert!(config.validate().is_ok());
}
