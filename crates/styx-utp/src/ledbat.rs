use std::time::Duration;

use crate::TARGET_DELAY;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedbatController {
    congestion_window: usize,
    min_window: usize,
    remote_window: usize,
    base_delay: Option<Duration>,
    current_delay: Option<Duration>,
}

impl LedbatController {
    #[must_use]
    pub const fn new(initial_window: usize, min_window: usize) -> Self {
        Self {
            congestion_window: initial_window,
            min_window,
            remote_window: usize::MAX,
            base_delay: None,
            current_delay: None,
        }
    }

    pub fn set_remote_window(&mut self, remote_window: usize) {
        self.remote_window = remote_window;
    }

    pub fn on_delay_sample(&mut self, delay: Duration, bytes_acked: usize, app_limited: bool) {
        self.base_delay = Some(self.base_delay.map_or(delay, |base| base.min(delay)));
        self.current_delay = Some(delay);
        if app_limited || bytes_acked == 0 {
            return;
        }

        let queuing = self.queuing_delay().unwrap_or_default();
        if queuing < TARGET_DELAY {
            let gain = (TARGET_DELAY - queuing).as_micros().max(1) as usize;
            self.congestion_window +=
                (bytes_acked * gain / TARGET_DELAY.as_micros() as usize).max(1);
        } else {
            let over = (queuing - TARGET_DELAY).as_micros().max(1) as usize;
            let decrease = (bytes_acked * over / TARGET_DELAY.as_micros() as usize).max(1);
            self.congestion_window = self
                .congestion_window
                .saturating_sub(decrease)
                .max(self.min_window);
        }
    }

    pub fn on_loss(&mut self) {
        self.congestion_window = (self.congestion_window / 2).max(self.min_window);
    }

    #[must_use]
    pub const fn congestion_window(&self) -> usize {
        self.congestion_window
    }

    #[must_use]
    pub fn allowed_window(&self) -> usize {
        self.congestion_window.min(self.remote_window)
    }

    #[must_use]
    pub fn can_send(&self, bytes_in_flight: usize, payload_len: usize) -> bool {
        bytes_in_flight + payload_len <= self.allowed_window()
    }

    #[must_use]
    pub const fn base_delay(&self) -> Option<Duration> {
        self.base_delay
    }

    #[must_use]
    pub const fn queuing_delay(&self) -> Option<Duration> {
        match (self.base_delay, self.current_delay) {
            (Some(base), Some(current)) => Some(current.saturating_sub(base)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn low_queuing_delay_increases_congestion_window() {
        let mut controller = LedbatController::new(1000, 120);
        controller.on_delay_sample(Duration::from_millis(50), 100, false);
        let before = controller.congestion_window();
        controller.on_delay_sample(Duration::from_millis(60), 100, false);

        assert!(controller.congestion_window() > before);
    }

    #[test]
    fn delay_above_target_decreases_window() {
        let mut controller = LedbatController::new(1000, 120);
        controller.on_delay_sample(Duration::from_millis(10), 1, false);
        let before = controller.congestion_window();
        controller.on_delay_sample(Duration::from_millis(200), 100, false);

        assert!(controller.congestion_window() < before);
    }

    #[test]
    fn allowed_window_respects_remote_window() {
        let mut controller = LedbatController::new(1000, 120);
        controller.set_remote_window(300);

        assert!(!controller.can_send(250, 100));
    }

    #[test]
    fn base_delay_keeps_minimum_observed_delay() {
        let mut controller = LedbatController::new(1000, 120);
        controller.on_delay_sample(Duration::from_millis(50), 100, false);
        controller.on_delay_sample(Duration::from_millis(20), 100, false);

        assert_eq!(controller.base_delay(), Some(Duration::from_millis(20)));
    }

    #[test]
    fn app_limited_sample_does_not_inflate_window() {
        let mut controller = LedbatController::new(1000, 120);
        controller.on_delay_sample(Duration::from_millis(50), 100, true);

        assert_eq!(controller.congestion_window(), 1000);
    }

    #[test]
    fn loss_halves_window_and_respects_minimum() {
        let mut controller = LedbatController::new(1000, 600);

        controller.on_loss();

        assert_eq!(controller.congestion_window(), 600);
    }
}
