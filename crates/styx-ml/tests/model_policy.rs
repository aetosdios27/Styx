use styx_ml::{
    FeatureVector, MlError, PolicyThresholds, StaticModel, ThrottleAction, ThrottleModel,
    ThrottleSignal, FEATURE_SCHEMA_VERSION,
};

#[test]
fn throttle_signal_rejects_probability_outside_unit_interval() {
    let err = ThrottleSignal::new(1.2, 10.0).unwrap_err();

    assert_eq!(err, MlError::InvalidProbability(1.2));
}

#[test]
fn static_model_validates_feature_schema_before_predicting() {
    let model = StaticModel::new(
        FEATURE_SCHEMA_VERSION,
        12,
        ThrottleSignal::new(0.5, 30.0).unwrap(),
    );
    let features = FeatureVector::new(FEATURE_SCHEMA_VERSION + 1, vec![0.0; 12]).unwrap();

    let err = model.predict(&features).unwrap_err();

    assert_eq!(
        err,
        MlError::SchemaMismatch {
            expected: FEATURE_SCHEMA_VERSION,
            actual: FEATURE_SCHEMA_VERSION + 1,
        }
    );
}

#[test]
fn policy_maps_medium_probability_to_rate_reduction_and_utp_preference() {
    let policy = PolicyThresholds::default();
    let signal = ThrottleSignal::new(0.5, 45.0).unwrap();

    let decision = policy.decide(signal);

    assert_eq!(
        decision.actions,
        vec![ThrottleAction::ReduceRequestRate, ThrottleAction::PreferUtp]
    );
}

#[test]
fn policy_maps_high_probability_to_identity_and_reconnect_advice() {
    let policy = PolicyThresholds::default();
    let signal = ThrottleSignal::new(0.9, 5.0).unwrap();

    let decision = policy.decide(signal);

    assert_eq!(
        decision.actions,
        vec![
            ThrottleAction::ReduceRequestRate,
            ThrottleAction::PreferUtp,
            ThrottleAction::RotatePeerId,
            ThrottleAction::DropAndReconnect,
        ]
    );
}
