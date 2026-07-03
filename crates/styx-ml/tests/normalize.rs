use styx_ml::{FeatureNormalizer, FeatureVector, MlError, FEATURE_SCHEMA_VERSION};

#[test]
fn normalizer_rejects_mismatched_parameter_lengths() {
    let err =
        FeatureNormalizer::new(FEATURE_SCHEMA_VERSION, vec![0.0], vec![1.0, 1.0]).unwrap_err();

    assert_eq!(
        err,
        MlError::FeatureLength {
            expected: 1,
            actual: 2
        }
    );
}

#[test]
fn normalizer_rejects_zero_stddev() {
    let err = FeatureNormalizer::new(FEATURE_SCHEMA_VERSION, vec![0.0], vec![0.0]).unwrap_err();

    assert_eq!(err, MlError::InvalidStdDev { index: 0 });
}

#[test]
fn normalizer_applies_mean_and_stddev_per_feature() {
    let normalizer =
        FeatureNormalizer::new(FEATURE_SCHEMA_VERSION, vec![10.0, 20.0], vec![2.0, 5.0]).unwrap();
    let features = FeatureVector::new(FEATURE_SCHEMA_VERSION, vec![14.0, 5.0]).unwrap();

    let normalized = normalizer.normalize(&features).unwrap();

    assert_eq!(normalized.values, vec![2.0, -3.0]);
}
