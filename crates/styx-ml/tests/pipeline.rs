use std::time::Duration;

use styx_ml::{
    ConnectionSample, FeatureNormalizer, StaticModel, ThrottleAction, ThrottleDetector,
    ThrottleSignal, FEATURE_NAMES, FEATURE_SCHEMA_VERSION,
};

#[test]
fn detector_pipeline_extracts_normalizes_predicts_and_maps_actions() {
    let normalizer =
        FeatureNormalizer::identity(FEATURE_SCHEMA_VERSION, FEATURE_NAMES.len()).unwrap();
    let model = StaticModel::new(
        FEATURE_SCHEMA_VERSION,
        FEATURE_NAMES.len(),
        ThrottleSignal::new(0.8, 10.0).unwrap(),
    );
    let mut detector = ThrottleDetector::new(model, normalizer);

    detector
        .observe(ConnectionSample {
            at: Duration::from_secs(1),
            bytes_down: 1024,
            bytes_up: 64,
            packet_gap: Some(Duration::from_millis(25)),
            rtt: Some(Duration::from_millis(80)),
            retransmits: 1,
        })
        .unwrap();
    let decision = detector.evaluate().unwrap();

    assert!(decision.actions.contains(&ThrottleAction::DropAndReconnect));
}
