use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use crate::RuntimeError;

#[derive(Clone, Debug)]
pub struct RateCounter {
    window: Duration,
    samples: VecDeque<RateSample>,
}

#[derive(Clone, Copy, Debug)]
struct RateSample {
    at: Instant,
    bytes: u64,
}

impl RateCounter {
    pub fn new(window: Duration) -> Result<Self, RuntimeError> {
        if window.is_zero() {
            return Err(RuntimeError::InvalidConfig(
                "rate window must be greater than zero",
            ));
        }
        Ok(Self {
            window,
            samples: VecDeque::new(),
        })
    }

    pub fn record(&mut self, at: Instant, bytes: u64) {
        self.samples.push_back(RateSample { at, bytes });
        self.prune(at);
    }

    #[must_use]
    pub fn bytes_per_second(&mut self, now: Instant) -> u64 {
        self.prune(now);
        let bytes = self.samples.iter().map(|sample| sample.bytes).sum::<u64>();
        bytes / self.window.as_secs().max(1)
    }

    fn prune(&mut self, now: Instant) {
        while self
            .samples
            .front()
            .is_some_and(|sample| now.duration_since(sample.at) > self.window)
        {
            self.samples.pop_front();
        }
    }
}
