use sha2::{Digest, Sha256};

use crate::MlError;

/// Supported model roles at the Styx ML boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    Classifier,
    Survival,
    Combined,
}

/// Metadata required before loading model bytes into a runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelManifest {
    pub model_kind: ModelKind,
    pub schema_version: u32,
    pub input_name: String,
    pub probability_output_name: String,
    pub onset_output_name: String,
    pub sha256: [u8; 32],
}

impl ModelManifest {
    pub fn new(
        model_kind: ModelKind,
        schema_version: u32,
        input_name: impl Into<String>,
        probability_output_name: impl Into<String>,
        onset_output_name: impl Into<String>,
        sha256: [u8; 32],
    ) -> Result<Self, MlError> {
        let manifest = Self {
            model_kind,
            schema_version,
            input_name: input_name.into(),
            probability_output_name: probability_output_name.into(),
            onset_output_name: onset_output_name.into(),
            sha256,
        };

        if manifest.input_name.is_empty()
            || manifest.probability_output_name.is_empty()
            || manifest.onset_output_name.is_empty()
        {
            return Err(MlError::EmptyTensorName);
        }

        Ok(manifest)
    }

    pub fn validate_model_bytes(&self, bytes: &[u8]) -> Result<(), MlError> {
        let digest = Sha256::digest(bytes);
        if digest.as_slice() == self.sha256 {
            Ok(())
        } else {
            Err(MlError::ModelChecksumMismatch)
        }
    }
}
