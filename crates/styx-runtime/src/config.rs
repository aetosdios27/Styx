use std::time::Duration;

use styx_core::PeerManagerConfig;

use crate::RuntimeError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeConfig {
    pub connect_timeout: Duration,
    pub source_timeout: Duration,
    pub piece_timeout: Duration,
    pub snapshot_interval: Duration,
    pub limits: RuntimeLimits,
    pub peer: PeerManagerConfig,
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

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            source_timeout: Duration::from_secs(15),
            piece_timeout: Duration::from_secs(30),
            snapshot_interval: Duration::from_secs(1),
            limits: RuntimeLimits::default(),
            peer: PeerManagerConfig::default(),
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

impl RuntimeConfig {
    pub fn validate(self) -> Result<Self, RuntimeError> {
        validate_duration(self.connect_timeout, "connect_timeout")?;
        validate_duration(self.source_timeout, "source_timeout")?;
        validate_duration(self.piece_timeout, "piece_timeout")?;
        validate_duration(self.snapshot_interval, "snapshot_interval")?;
        self.limits.validate()?;
        self.peer.validate().map_err(|err| match err {
            styx_core::CoreError::InvalidConfig { field } => {
                RuntimeError::InvalidConfig(peer_config_message(field))
            }
            _ => RuntimeError::InvalidConfig("peer manager config is invalid"),
        })?;
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
        "max_active_torrents" => "max_active_torrents must be greater than zero",
        "max_peers_per_torrent" => "max_peers_per_torrent must be greater than zero",
        "max_sources_per_torrent" => "max_sources_per_torrent must be greater than zero",
        "max_web_seed_concurrency" => "max_web_seed_concurrency must be greater than zero",
        "max_event_queue" => "max_event_queue must be greater than zero",
        "source_retry_limit" => "source_retry_limit must be greater than zero",
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
