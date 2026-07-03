//! Privacy-preserving throttling detection primitives for Styx.
//!
//! This crate owns model-independent telemetry shaping, feature extraction,
//! normalization, model boundaries, and advisory throttling policy. It does not
//! store peer identities, mutate peer state, or perform network I/O.

mod error;
mod features;
mod manifest;
mod model;
mod normalize;
mod pipeline;
mod policy;
mod types;
mod window;

pub use error::MlError;
pub use features::{FeatureExtractor, FEATURE_NAMES, FEATURE_SCHEMA_VERSION};
pub use manifest::{ModelKind, ModelManifest};
pub use model::{NoopModel, StaticModel, ThrottleModel};
pub use normalize::FeatureNormalizer;
pub use pipeline::ThrottleDetector;
pub use policy::PolicyThresholds;
pub use types::{
    ConnectionSample, FeatureVector, ThrottleAction, ThrottleDecision, ThrottleSignal,
};
pub use window::RollingWindow;
