use std::time::Duration;

use styx_ml::{ConnectionSample, RollingWindow};

fn sample(at_secs: u64, bytes_down: u64, bytes_up: u64) -> ConnectionSample {
    ConnectionSample {
        at: Duration::from_secs(at_secs),
        bytes_down,
        bytes_up,
        packet_gap: None,
        rtt: None,
        retransmits: 0,
    }
}

#[test]
fn rolling_window_prunes_samples_older_than_horizon() {
    let mut window = RollingWindow::new(Duration::from_secs(10));

    window.push(sample(0, 100, 10));
    window.push(sample(5, 200, 20));
    window.push(sample(11, 300, 30));

    assert_eq!(window.bytes_down(), 500);
}

#[test]
fn rolling_window_reports_rate_over_configured_horizon() {
    let mut window = RollingWindow::new(Duration::from_secs(5));

    window.push(sample(1, 500, 250));

    assert_eq!(window.down_rate_bps(), 100.0);
}
