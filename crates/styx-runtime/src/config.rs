use std::time::Duration;

use styx_core::PeerManagerConfig;

use crate::{DhtRuntimeConfig, RuntimeError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeConfig {
    pub connect_timeout: Duration,
    pub source_timeout: Duration,
    pub piece_timeout: Duration,
    pub snapshot_interval: Duration,
    pub listen_port: u16,
    pub limits: RuntimeLimits,
    pub peer: PeerManagerConfig,
    pub seed_policy: SeedPolicy,
    pub dht: DhtRuntimeConfig,
    pub session: SessionRuntimeConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionRuntimeConfig {
    pub command_capacity: usize,
    pub event_capacity: usize,
    pub shutdown_timeout: Duration,
    pub forced_shutdown_timeout: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeLimits {
    pub max_active_torrents: usize,
    pub max_peers_per_torrent: usize,
    pub max_sources_per_torrent: usize,
    pub max_web_seed_concurrency: usize,
    pub max_event_queue: usize,
    pub source_retry_limit: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeedPolicy {
    pub seed_after_complete: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            source_timeout: Duration::from_secs(15),
            piece_timeout: Duration::from_secs(30),
            snapshot_interval: Duration::from_secs(1),
            listen_port: 6881,
            limits: RuntimeLimits::default(),
            peer: PeerManagerConfig::default(),
            seed_policy: SeedPolicy::default(),
            dht: DhtRuntimeConfig::default(),
            session: SessionRuntimeConfig::default(),
        }
    }
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_active_torrents: 8,
            max_peers_per_torrent: 30,
            max_sources_per_torrent: 64,
            max_web_seed_concurrency: 2,
            max_event_queue: 1024,
            source_retry_limit: 3,
        }
    }
}

impl Default for SeedPolicy {
    fn default() -> Self {
        Self {
            seed_after_complete: true,
        }
    }
}

impl Default for SessionRuntimeConfig {
    fn default() -> Self {
        Self {
            command_capacity: 256,
            event_capacity: 1024,
            shutdown_timeout: Duration::from_secs(10),
            forced_shutdown_timeout: Duration::from_secs(2),
        }
    }
}

impl RuntimeConfig {
    pub fn validate(self) -> Result<Self, RuntimeError> {
        validate_duration(self.connect_timeout, "connect_timeout")?;
        validate_duration(self.source_timeout, "source_timeout")?;
        validate_duration(self.piece_timeout, "piece_timeout")?;
        validate_duration(self.snapshot_interval, "snapshot_interval")?;
        validate_nonzero(self.listen_port as usize, "listen_port")?;
        self.limits.validate()?;
        self.peer.validate().map_err(|err| match err {
            styx_core::CoreError::InvalidConfig { field } => {
                RuntimeError::InvalidConfig(peer_config_message(field))
            }
            _ => RuntimeError::InvalidConfig("peer manager config is invalid"),
        })?;
        self.dht.validate()?;
        self.session.validate()?;
        Ok(self)
    }
}

impl SessionRuntimeConfig {
    pub fn validate(self) -> Result<Self, RuntimeError> {
        validate_nonzero(self.command_capacity, "session_command_capacity")?;
        validate_nonzero(self.event_capacity, "session_event_capacity")?;
        validate_duration(self.shutdown_timeout, "session_shutdown_timeout")?;
        validate_duration(
            self.forced_shutdown_timeout,
            "session_forced_shutdown_timeout",
        )?;
        Ok(self)
    }
}

impl RuntimeLimits {
    pub fn validate(self) -> Result<Self, RuntimeError> {
        validate_nonzero(self.max_active_torrents, "max_active_torrents")?;
        validate_nonzero(self.max_peers_per_torrent, "max_peers_per_torrent")?;
        validate_nonzero(self.max_sources_per_torrent, "max_sources_per_torrent")?;
        validate_nonzero(self.max_web_seed_concurrency, "max_web_seed_concurrency")?;
        validate_nonzero(self.max_event_queue, "max_event_queue")?;
        validate_nonzero(self.source_retry_limit, "source_retry_limit")?;
        Ok(self)
    }
}

fn validate_duration(value: Duration, field: &'static str) -> Result<(), RuntimeError> {
    if value.is_zero() {
        return Err(invalid_nonzero(field));
    }
    Ok(())
}

fn validate_nonzero(value: usize, field: &'static str) -> Result<(), RuntimeError> {
    if value == 0 {
        return Err(invalid_nonzero(field));
    }
    Ok(())
}

fn invalid_nonzero(field: &'static str) -> RuntimeError {
    RuntimeError::InvalidConfig(match field {
        "connect_timeout" => "connect_timeout must be greater than zero",
        "source_timeout" => "source_timeout must be greater than zero",
        "piece_timeout" => "piece_timeout must be greater than zero",
        "snapshot_interval" => "snapshot_interval must be greater than zero",
        "listen_port" => "listen_port must be greater than zero",
        "max_active_torrents" => "max_active_torrents must be greater than zero",
        "max_peers_per_torrent" => "max_peers_per_torrent must be greater than zero",
        "max_sources_per_torrent" => "max_sources_per_torrent must be greater than zero",
        "max_web_seed_concurrency" => "max_web_seed_concurrency must be greater than zero",
        "max_event_queue" => "max_event_queue must be greater than zero",
        "source_retry_limit" => "source_retry_limit must be greater than zero",
        "session_command_capacity" => "session command capacity must be greater than zero",
        "session_event_capacity" => "session event capacity must be greater than zero",
        "session_shutdown_timeout" => "session shutdown timeout must be greater than zero",
        "session_forced_shutdown_timeout" => {
            "session forced shutdown timeout must be greater than zero"
        }
        _ => "runtime config value must be greater than zero",
    })
}

fn peer_config_message(field: &'static str) -> &'static str {
    match field {
        "upload_slots" => "upload_slots must be greater than zero",
        "request_pipeline_depth" => "request_pipeline_depth must be greater than zero",
        "choke_interval" => "choke_interval must be greater than zero",
        "optimistic_unchoke_interval" => "optimistic_unchoke_interval must be greater than zero",
        "rate_window" => "rate_window must be greater than zero",
        "request_timeout" => "request_timeout must be greater than zero",
        _ => "peer manager config is invalid",
    }
}
