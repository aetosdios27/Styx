use std::time::Duration;

use crate::{INITIAL_TIMEOUT, MIN_TIMEOUT};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RttEstimator {
    srtt: Option<Duration>,
    rttvar: Duration,
}

impl Default for RttEstimator {
    fn default() -> Self {
        Self {
            srtt: None,
            rttvar: Duration::ZERO,
        }
    }
}

impl RttEstimator {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            srtt: None,
            rttvar: Duration::ZERO,
        }
    }

    pub fn sample(&mut self, rtt: Duration, retransmitted: bool) {
        if retransmitted {
            return;
        }
        if let Some(srtt) = self.srtt {
            let diff = srtt.abs_diff(rtt);
            self.rttvar = mul_div_duration(self.rttvar, 3, 4) + mul_div_duration(diff, 1, 4);
            self.srtt = Some(mul_div_duration(srtt, 7, 8) + mul_div_duration(rtt, 1, 8));
        } else {
            self.srtt = Some(rtt);
            self.rttvar = mul_div_duration(rtt, 1, 2);
        }
    }

    #[must_use]
    pub fn timeout(&self) -> Duration {
        let Some(srtt) = self.srtt else {
            return INITIAL_TIMEOUT;
        };
        (srtt + self.rttvar * 4).max(MIN_TIMEOUT)
    }
}

fn mul_div_duration(duration: Duration, numerator: u32, denominator: u32) -> Duration {
    Duration::from_micros(
        (duration.as_micros() * u128::from(numerator) / u128::from(denominator)) as u64,
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn sample_from_once_sent_packet_updates_timeout() {
        let mut estimator = RttEstimator::new();

        estimator.sample(Duration::from_millis(100), false);

        assert_eq!(estimator.timeout(), Duration::from_millis(500));
    }

    #[test]
    fn retransmitted_packet_does_not_update_estimator() {
        let mut estimator = RttEstimator::new();

        estimator.sample(Duration::from_millis(100), true);

        assert_eq!(estimator.timeout(), INITIAL_TIMEOUT);
    }

    #[test]
    fn timeout_uses_srtt_plus_four_rttvar_after_samples() {
        let mut estimator = RttEstimator::new();
        estimator.sample(Duration::from_millis(400), false);

        assert_eq!(estimator.timeout(), Duration::from_millis(1200));
    }

    #[test]
    fn initial_timeout_is_one_second() {
        assert_eq!(RttEstimator::new().timeout(), Duration::from_secs(1));
    }
}
