use styx_cli::{
    commands::{CommandEnvelope, CommandResponseEnvelope, ControlCommand},
    ipc::{decode_command, encode_command, encode_response},
};

#[test]
fn command_codec_rejects_trailing_json() {
    let err = decode_command(br#"{"type":"status"}{"type":"status"}"#).unwrap_err();

    assert!(err.to_string().contains("trailing"));
}

#[test]
fn command_codec_round_trips_one_command_per_line() {
    let encoded = encode_command(&ControlCommand::Status).unwrap();

    let decoded = decode_command(&encoded).unwrap();

    assert_eq!(decoded, ControlCommand::Status);
}

#[test]
fn response_envelope_serializes_failure() {
    let encoded = encode_response(&CommandResponseEnvelope::err("bad command")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();

    assert_eq!(value["ok"], false);
}

#[test]
fn command_envelope_defaults_to_current_protocol_version() {
    let envelope = CommandEnvelope::new(ControlCommand::Status);

    assert_eq!(envelope.version, 1);
}
