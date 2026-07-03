use std::time::Duration;

use styx_ml::{ConnectionSample, FeatureExtractor, MlError, FEATURE_NAMES, FEATURE_SCHEMA_VERSION};

fn sample(
    at_secs: u64,
    bytes_down: u64,
    bytes_up: u64,
    packet_gap_ms: Option<u64>,
    rtt_ms: Option<u64>,
    retransmits: u32,
) -> ConnectionSample {
    ConnectionSample {
        at: Duration::from_secs(at_secs),
        bytes_down,
        bytes_up,
        packet_gap: packet_gap_ms.map(Duration::from_millis),
        rtt: rtt_ms.map(Duration::from_millis),
        retransmits,
    }
}

#[test]
fn feature_extractor_emits_stable_schema_length_and_version() {
    let mut extractor = FeatureExtractor::new();

    extractor
        .observe(sample(0, 1_000, 100, Some(10), Some(50), 1))
        .unwrap();
    let features = extractor.extract().unwrap();

    assert_eq!(
        (features.schema_version, features.values.len()),
        (FEATURE_SCHEMA_VERSION, FEATURE_NAMES.len())
    );
}

#[test]
fn feature_extractor_computes_throughput_and_age_features() {
    let mut extractor = FeatureExtractor::new();

    extractor
        .observe(sample(0, 500, 50, Some(10), Some(50), 1))
        .unwrap();
    extractor
        .observe(sample(6, 1_000, 100, Some(20), Some(70), 2))
        .unwrap();
    let features = extractor.extract().unwrap();

    assert_eq!(
        (
            features.values[0],
            features.values[1],
            features.values[3],
            features.values[5],
        ),
        (200.0, 50.0, 5.0, 6.0)
    );
}

#[test]
fn feature_extractor_never_emits_non_finite_statistics_for_missing_observations() {
    let mut extractor = FeatureExtractor::new();

    extractor.observe(sample(1, 0, 0, None, None, 0)).unwrap();
    let features = extractor.extract().unwrap();

    assert!(features.values.iter().all(|value| value.is_finite()));
}

#[test]
fn feature_extractor_rejects_non_monotonic_samples() {
    let mut extractor = FeatureExtractor::new();

    extractor.observe(sample(10, 1, 1, None, None, 0)).unwrap();
    let err = extractor
        .observe(sample(9, 1, 1, None, None, 0))
        .unwrap_err();

    assert_eq!(err, MlError::NonMonotonicSample);
}
