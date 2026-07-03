use std::time::Duration;

use crate::{ConnectionSample, FeatureVector, MlError, RollingWindow};

/// Current feature schema version.
pub const FEATURE_SCHEMA_VERSION: u32 = 1;

/// Ordered feature names. This order is the model input contract.
pub const FEATURE_NAMES: [&str; 12] = [
    "throughput_down_5s_bps",
    "throughput_down_30s_bps",
    "throughput_down_120s_bps",
    "throughput_up_30s_bps",
    "upload_download_ratio_30s",
    "connection_age_seconds",
    "rtt_mean_ms",
    "rtt_variance_ms2",
    "packet_gap_mean_ms",
    "packet_gap_variance_ms2",
    "packet_gap_skewness",
    "retransmits_30s",
];

const RATIO_CAP: f32 = 1_000.0;

/// Extracts stable, finite features from session-local connection telemetry.
#[derive(Clone, Debug)]
pub struct FeatureExtractor {
    window_5s: RollingWindow,
    window_30s: RollingWindow,
    window_120s: RollingWindow,
    first_seen: Option<Duration>,
    last_seen: Option<Duration>,
}

impl Default for FeatureExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureExtractor {
    pub fn new() -> Self {
        Self {
            window_5s: RollingWindow::new(Duration::from_secs(5)),
            window_30s: RollingWindow::new(Duration::from_secs(30)),
            window_120s: RollingWindow::new(Duration::from_secs(120)),
            first_seen: None,
            last_seen: None,
        }
    }

    pub fn observe(&mut self, sample: ConnectionSample) -> Result<(), MlError> {
        if self
            .last_seen
            .is_some_and(|last_seen| sample.at < last_seen)
        {
            return Err(MlError::NonMonotonicSample);
        }

        self.first_seen.get_or_insert(sample.at);
        self.last_seen = Some(sample.at);
        self.window_5s.push(sample.clone());
        self.window_30s.push(sample.clone());
        self.window_120s.push(sample);

        Ok(())
    }

    pub fn extract(&self) -> Result<FeatureVector, MlError> {
        let down_30 = self.window_30s.bytes_down();
        let up_30 = self.window_30s.bytes_up();
        let rtts = self.window_120s.rtts_ms();
        let packet_gaps = self.window_120s.packet_gaps_ms();
        let values = vec![
            self.window_5s.down_rate_bps(),
            self.window_30s.down_rate_bps(),
            self.window_120s.down_rate_bps(),
            self.window_30s.up_rate_bps(),
            upload_download_ratio(up_30, down_30),
            self.connection_age_seconds(),
            mean(&rtts),
            variance(&rtts),
            mean(&packet_gaps),
            variance(&packet_gaps),
            skewness(&packet_gaps),
            self.window_30s.retransmits() as f32,
        ];

        FeatureVector::new(FEATURE_SCHEMA_VERSION, values)
    }

    fn connection_age_seconds(&self) -> f32 {
        match (self.first_seen, self.last_seen) {
            (Some(first), Some(last)) => last.saturating_sub(first).as_secs_f32(),
            _ => 0.0,
        }
    }
}

fn upload_download_ratio(bytes_up: u64, bytes_down: u64) -> f32 {
    match (bytes_up, bytes_down) {
        (0, 0) => 0.0,
        (_, 0) => RATIO_CAP,
        (up, down) => up as f32 / down as f32,
    }
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }

    values.iter().sum::<f32>() / values.len() as f32
}

fn variance(values: &[f32]) -> f32 {
    if values.len() < 2 {
        return 0.0;
    }

    let avg = mean(values);
    values
        .iter()
        .map(|value| {
            let delta = value - avg;
            delta * delta
        })
        .sum::<f32>()
        / values.len() as f32
}

fn skewness(values: &[f32]) -> f32 {
    if values.len() < 2 {
        return 0.0;
    }

    let avg = mean(values);
    let variance = variance(values);
    if variance == 0.0 {
        return 0.0;
    }

    let stddev = variance.sqrt();
    values
        .iter()
        .map(|value| ((value - avg) / stddev).powi(3))
        .sum::<f32>()
        / values.len() as f32
}
