use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use crate::CoreError;

#[derive(Clone, Debug)]
pub struct RateWindow {
    window: Duration,
    samples: VecDeque<(Instant, u64)>,
}

impl RateWindow {
    pub fn new(window: Duration) -> Result<Self, CoreError> {
        if window.is_zero() {
            return Err(CoreError::InvalidConfig {
                field: "rate_window",
            });
        }
        Ok(Self {
            window,
            samples: VecDeque::new(),
        })
    }

    pub fn record(&mut self, now: Instant, bytes: u64) {
        self.prune(now);
        self.samples.push_back((now, bytes));
    }

    #[must_use]
    pub fn bytes_per_second(&mut self, now: Instant) -> u64 {
        self.prune(now);
        let total = self.samples.iter().map(|(_, bytes)| *bytes).sum::<u64>();
        total / self.window.as_secs()
    }

    fn prune(&mut self, now: Instant) {
        while self
            .samples
            .front()
            .is_some_and(|(sample_at, _)| now.duration_since(*sample_at) > self.window)
        {
            self.samples.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::{CoreError, RateWindow};

    #[test]
    fn bytes_per_second_excludes_samples_older_than_window() {
        let start = Instant::now();
        let mut window = RateWindow::new(Duration::from_secs(20)).unwrap();
        window.record(start, 1_000);
        window.record(start + Duration::from_secs(10), 2_000);

        let rate = window.bytes_per_second(start + Duration::from_secs(21));

        assert_eq!(rate, 100);
    }

    #[test]
    fn bytes_per_second_counts_all_samples_inside_window() {
        let start = Instant::now();
        let mut window = RateWindow::new(Duration::from_secs(20)).unwrap();
        window.record(start + Duration::from_secs(1), 1_000);
        window.record(start + Duration::from_secs(5), 3_000);

        let rate = window.bytes_per_second(start + Duration::from_secs(10));

        assert_eq!(rate, 200);
    }

    #[test]
    fn new_rejects_zero_window() {
        let err = RateWindow::new(Duration::ZERO).unwrap_err();

        assert_eq!(
            err,
            CoreError::InvalidConfig {
                field: "rate_window"
            }
        );
    }
}
