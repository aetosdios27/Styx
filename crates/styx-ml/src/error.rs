use thiserror::Error;

/// Errors returned by throttling detection components.
#[derive(Debug, Error, PartialEq)]
pub enum MlError {
    #[error("feature length mismatch: expected {expected}, got {actual}")]
    FeatureLength { expected: usize, actual: usize },

    #[error("schema mismatch: expected {expected}, got {actual}")]
    SchemaMismatch { expected: u32, actual: u32 },

    #[error("non-finite feature at index {index}")]
    NonFiniteFeature { index: usize },

    #[error("invalid standard deviation at index {index}")]
    InvalidStdDev { index: usize },

    #[error("invalid probability {0}")]
    InvalidProbability(f32),

    #[error("invalid onset seconds {0}")]
    InvalidOnset(f32),

    #[error("empty tensor name")]
    EmptyTensorName,

    #[error("model checksum mismatch")]
    ModelChecksumMismatch,

    #[error("invalid policy thresholds")]
    InvalidThresholds,

    #[error("connection samples must be monotonic")]
    NonMonotonicSample,
}
