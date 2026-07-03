use crate::{FeatureVector, MlError};

/// Per-feature normalization parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureNormalizer {
    schema_version: u32,
    means: Vec<f32>,
    stddevs: Vec<f32>,
}

impl FeatureNormalizer {
    pub fn new(schema_version: u32, means: Vec<f32>, stddevs: Vec<f32>) -> Result<Self, MlError> {
        if means.len() != stddevs.len() {
            return Err(MlError::FeatureLength {
                expected: means.len(),
                actual: stddevs.len(),
            });
        }

        for (index, value) in means.iter().enumerate() {
            if !value.is_finite() {
                return Err(MlError::NonFiniteFeature { index });
            }
        }

        for (index, value) in stddevs.iter().enumerate() {
            if !value.is_finite() || *value <= 0.0 {
                return Err(MlError::InvalidStdDev { index });
            }
        }

        Ok(Self {
            schema_version,
            means,
            stddevs,
        })
    }

    pub fn identity(schema_version: u32, len: usize) -> Result<Self, MlError> {
        Self::new(schema_version, vec![0.0; len], vec![1.0; len])
    }

    pub fn normalize(&self, features: &FeatureVector) -> Result<FeatureVector, MlError> {
        if features.schema_version != self.schema_version {
            return Err(MlError::SchemaMismatch {
                expected: self.schema_version,
                actual: features.schema_version,
            });
        }
        if features.values.len() != self.means.len() {
            return Err(MlError::FeatureLength {
                expected: self.means.len(),
                actual: features.values.len(),
            });
        }

        let values = features
            .values
            .iter()
            .zip(self.means.iter().zip(self.stddevs.iter()))
            .map(|(value, (mean, stddev))| (value - mean) / stddev)
            .collect();

        FeatureVector::new(features.schema_version, values)
    }
}
