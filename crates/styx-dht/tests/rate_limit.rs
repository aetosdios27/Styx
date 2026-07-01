use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use styx_dht::SourceRateLimiter;

#[test]
fn source_rate_limiter_allows_capacity_inside_window() {
    let mut limiter = SourceRateLimiter::new(2, Duration::from_secs(1));
    let now = Instant::now();
    let source = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));

    assert!(limiter.check(source, now));
    assert!(limiter.check(source, now));
}

#[test]
fn source_rate_limiter_rejects_source_past_capacity() {
    let mut limiter = SourceRateLimiter::new(1, Duration::from_secs(1));
    let now = Instant::now();
    let source = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 2));
    assert!(limiter.check(source, now));

    assert!(!limiter.check(source, now));
}

#[test]
fn source_rate_limiter_resets_after_window() {
    let mut limiter = SourceRateLimiter::new(1, Duration::from_secs(1));
    let now = Instant::now();
    let source = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 3));
    assert!(limiter.check(source, now));

    assert!(limiter.check(source, now + Duration::from_secs(2)));
}
