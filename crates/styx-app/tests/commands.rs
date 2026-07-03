use styx_app::commands::{CommandEnvelope, CommandResponseEnvelope, ControlCommand};

#[test]
fn control_command_round_trips_as_tagged_json() {
    let command = ControlCommand::Status;

    let json = serde_json::to_string(&command).unwrap();
    let decoded: ControlCommand = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded, command);
}

#[test]
fn response_envelope_serializes_failure() {
    let encoded = serde_json::to_vec(&CommandResponseEnvelope::err("bad command")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();

    assert_eq!(value["ok"], false);
}

#[test]
fn command_envelope_defaults_to_current_protocol_version() {
    let envelope = CommandEnvelope::new(ControlCommand::Status);

    assert_eq!(envelope.version, 1);
}
