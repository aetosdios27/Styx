use crate::{FeatureVector, MlError, ThrottleSignal};

/// Runtime boundary for throttling models.
pub trait ThrottleModel {
    fn predict(&self, features: &FeatureVector) -> Result<ThrottleSignal, MlError>;
}

/// Deterministic model that always returns no throttling.
#[derive(Clone, Debug)]
pub struct NoopModel {
    schema_version: u32,
    feature_len: usize,
}

impl NoopModel {
    pub fn new(schema_version: u32, feature_len: usize) -> Self {
        Self {
            schema_version,
            feature_len,
        }
    }
}

impl ThrottleModel for NoopModel {
    fn predict(&self, features: &FeatureVector) -> Result<ThrottleSignal, MlError> {
        validate_features(features, self.schema_version, self.feature_len)?;
        ThrottleSignal::new(0.0, 0.0)
    }
}

/// Deterministic model useful for integration and policy tests.
#[derive(Clone, Debug)]
pub struct StaticModel {
    schema_version: u32,
    feature_len: usize,
    signal: ThrottleSignal,
}

impl StaticModel {
    pub fn new(schema_version: u32, feature_len: usize, signal: ThrottleSignal) -> Self {
        Self {
            schema_version,
            feature_len,
            signal,
        }
    }
}

impl ThrottleModel for StaticModel {
    fn predict(&self, features: &FeatureVector) -> Result<ThrottleSignal, MlError> {
        validate_features(features, self.schema_version, self.feature_len)?;
        Ok(self.signal)
    }
}

fn validate_features(
    features: &FeatureVector,
    schema_version: u32,
    feature_len: usize,
) -> Result<(), MlError> {
    if features.schema_version != schema_version {
        return Err(MlError::SchemaMismatch {
            expected: schema_version,
            actual: features.schema_version,
        });
    }
    if features.values.len() != feature_len {
        return Err(MlError::FeatureLength {
            expected: feature_len,
            actual: features.values.len(),
        });
    }

    Ok(())
}
