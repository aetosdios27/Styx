use styx_ml::{MlError, ModelKind, ModelManifest, FEATURE_SCHEMA_VERSION};

#[test]
fn manifest_rejects_empty_tensor_names() {
    let err = ModelManifest::new(
        ModelKind::Combined,
        FEATURE_SCHEMA_VERSION,
        "",
        "probability",
        "onset",
        [0; 32],
    )
    .unwrap_err();

    assert_eq!(err, MlError::EmptyTensorName);
}

#[test]
fn manifest_rejects_wrong_model_bytes_checksum() {
    let manifest = ModelManifest::new(
        ModelKind::Combined,
        FEATURE_SCHEMA_VERSION,
        "features",
        "probability",
        "onset",
        [0; 32],
    )
    .unwrap();

    let err = manifest
        .validate_model_bytes(b"not the expected model")
        .unwrap_err();

    assert_eq!(err, MlError::ModelChecksumMismatch);
}
