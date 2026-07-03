use std::{collections::VecDeque, time::Duration};

use crate::ConnectionSample;

/// Bounded rolling telemetry window.
#[derive(Clone, Debug)]
pub struct RollingWindow {
    horizon: Duration,
    samples: VecDeque<ConnectionSample>,
}

impl RollingWindow {
    /// Creates an empty rolling window with the given horizon.
    pub fn new(horizon: Duration) -> Self {
        Self {
            horizon,
            samples: VecDeque::new(),
        }
    }

    /// Adds a sample and prunes observations older than the horizon.
    pub fn push(&mut self, sample: ConnectionSample) {
        let cutoff = sample
            .at
            .checked_sub(self.horizon)
            .unwrap_or(Duration::ZERO);
        self.samples.push_back(sample);

        while self
            .samples
            .front()
            .is_some_and(|oldest| oldest.at < cutoff)
        {
            self.samples.pop_front();
        }
    }

    pub fn bytes_down(&self) -> u64 {
        self.samples.iter().map(|sample| sample.bytes_down).sum()
    }

    pub fn bytes_up(&self) -> u64 {
        self.samples.iter().map(|sample| sample.bytes_up).sum()
    }

    pub fn retransmits(&self) -> u64 {
        self.samples
            .iter()
            .map(|sample| u64::from(sample.retransmits))
            .sum()
    }

    pub fn down_rate_bps(&self) -> f32 {
        rate(self.bytes_down(), self.horizon)
    }

    pub fn up_rate_bps(&self) -> f32 {
        rate(self.bytes_up(), self.horizon)
    }

    pub(crate) fn rtts_ms(&self) -> Vec<f32> {
        self.samples
            .iter()
            .filter_map(|sample| sample.rtt.map(duration_ms))
            .collect()
    }

    pub(crate) fn packet_gaps_ms(&self) -> Vec<f32> {
        self.samples
            .iter()
            .filter_map(|sample| sample.packet_gap.map(duration_ms))
            .collect()
    }
}

fn rate(bytes: u64, horizon: Duration) -> f32 {
    let seconds = horizon.as_secs_f32();
    if seconds == 0.0 {
        0.0
    } else {
        bytes as f32 / seconds
    }
}

fn duration_ms(duration: Duration) -> f32 {
    duration.as_secs_f32() * 1_000.0
}
