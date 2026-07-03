use std::time::Duration;

use crate::MlError;

/// A session-local telemetry sample for one peer connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionSample {
    /// Monotonic time since the peer session started.
    pub at: Duration,
    /// Bytes received since the previous sample.
    pub bytes_down: u64,
    /// Bytes sent since the previous sample.
    pub bytes_up: u64,
    /// Observed inter-packet gap, when available.
    pub packet_gap: Option<Duration>,
    /// Observed round-trip time, when available.
    pub rtt: Option<Duration>,
    /// Retransmissions observed since the previous sample.
    pub retransmits: u32,
}

/// Ordered model features with an explicit schema version.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureVector {
    pub schema_version: u32,
    pub values: Vec<f32>,
}

impl FeatureVector {
    /// Creates a vector after rejecting non-finite values.
    pub fn new(schema_version: u32, values: Vec<f32>) -> Result<Self, MlError> {
        if let Some((index, _)) = values
            .iter()
            .enumerate()
            .find(|(_, value)| !value.is_finite())
        {
            return Err(MlError::NonFiniteFeature { index });
        }

        Ok(Self {
            schema_version,
            values,
        })
    }
}

/// Probability and timing estimate emitted by a throttling model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThrottleSignal {
    pub probability: f32,
    pub estimated_onset_seconds: f32,
}

impl ThrottleSignal {
    /// Creates a validated throttling signal.
    pub fn new(probability: f32, estimated_onset_seconds: f32) -> Result<Self, MlError> {
        if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
            return Err(MlError::InvalidProbability(probability));
        }
        if !estimated_onset_seconds.is_finite() || estimated_onset_seconds < 0.0 {
            return Err(MlError::InvalidOnset(estimated_onset_seconds));
        }

        Ok(Self {
            probability,
            estimated_onset_seconds,
        })
    }
}

/// Advisory actions for the caller. The ML crate never executes them directly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThrottleAction {
    NoAction,
    ReduceRequestRate,
    PreferUtp,
    RotatePeerId,
    DropAndReconnect,
}

/// Policy result derived from a validated model signal.
#[derive(Clone, Debug, PartialEq)]
pub struct ThrottleDecision {
    pub signal: ThrottleSignal,
    pub actions: Vec<ThrottleAction>,
}
