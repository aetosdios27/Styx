use std::net::{Ipv4Addr, Ipv6Addr};

use bytes::Bytes;
use styx_proto::{
    decode_extension_handshake, encode_extension_handshake, ExtensionError, ExtensionHandshake,
};

#[test]
fn extension_handshake_decodes_ut_metadata_and_metadata_size() {
    let decoded =
        decode_extension_handshake(b"d1:md11:ut_metadatai3ee13:metadata_sizei32768ee").unwrap();

    assert_eq!(decoded.message_id("ut_metadata"), Some(3));
    assert_eq!(decoded.metadata_size, Some(32_768));
}

#[test]
fn extension_handshake_ignores_unknown_keys() {
    let decoded = decode_extension_handshake(b"d1:md11:ut_metadatai4ee4:spaml4:eggsee").unwrap();

    assert_eq!(decoded.message_id("ut_metadata"), Some(4));
}

#[test]
fn extension_handshake_treats_zero_message_id_as_disabled() {
    let decoded = decode_extension_handshake(b"d1:md11:ut_metadatai0eee").unwrap();

    assert_eq!(decoded.message_id("ut_metadata"), None);
}

#[test]
fn extension_handshake_rejects_negative_message_id() {
    let err = decode_extension_handshake(b"d1:md11:ut_metadatai-1eee").unwrap_err();

    assert!(matches!(err, ExtensionError::InvalidMessageId { .. }));
}

#[test]
fn extension_handshake_rejects_oversized_message_id() {
    let err = decode_extension_handshake(b"d1:md11:ut_metadatai256eee").unwrap_err();

    assert!(matches!(err, ExtensionError::InvalidMessageId { .. }));
}

#[test]
fn extension_handshake_round_trips_listen_port_and_ipv6() {
    let handshake = ExtensionHandshake {
        messages: [("ut_metadata".to_string(), 7)].into(),
        metadata_size: Some(65_536),
        listen_port: Some(6881),
        client: Some("privacy-test-client".to_string()),
        ipv4: Some(Ipv4Addr::new(127, 0, 0, 1)),
        ipv6: Some(Ipv6Addr::LOCALHOST),
    };

    let decoded = decode_extension_handshake(&encode_extension_handshake(&handshake)).unwrap();

    assert_eq!(decoded, handshake);
}

#[test]
fn extension_handshake_rejects_wrong_metadata_size_type() {
    let err = decode_extension_handshake(b"d1:mde13:metadata_size4:bad!e").unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::InvalidFieldType {
            field: "metadata_size"
        }
    ));
}

#[test]
fn extension_handshake_rejects_wrong_ipv6_length() {
    let err = decode_extension_handshake(b"d4:ipv65:short1:mdee").unwrap_err();

    assert!(matches!(
        err,
        ExtensionError::InvalidFieldLength { field: "ipv6", .. }
    ));
}

#[test]
fn extension_handshake_rejects_non_dictionary_payload() {
    let err = decode_extension_handshake(b"4:spam").unwrap_err();

    assert!(matches!(err, ExtensionError::ExpectedDictionary));
}

#[test]
fn extension_handshake_encodes_zero_id_as_disabled_but_preserves_field() {
    let handshake = ExtensionHandshake {
        messages: [("ut_metadata".to_string(), 0)].into(),
        metadata_size: None,
        listen_port: None,
        client: None,
        ipv4: None,
        ipv6: None,
    };

    let decoded = decode_extension_handshake(&encode_extension_handshake(&handshake)).unwrap();

    assert_eq!(decoded.messages.get("ut_metadata"), Some(&0));
    assert_eq!(decoded.message_id("ut_metadata"), None);
}

#[test]
fn extension_handshake_accepts_yourip_as_ipv4_or_ipv6() {
    let decoded = decode_extension_handshake(b"d1:mde6:yourip4:\x7f\0\0\x01e").unwrap();

    assert_eq!(decoded.ipv4, Some(Ipv4Addr::new(127, 0, 0, 1)));
}

#[test]
fn extension_handshake_does_not_require_all_optional_fields() {
    let decoded = decode_extension_handshake(b"d1:mdee").unwrap();

    assert_eq!(decoded, ExtensionHandshake::default());
}

#[test]
fn extension_handshake_encodes_client_as_bytes() {
    let handshake = ExtensionHandshake {
        client: Some("Styx".to_string()),
        ..ExtensionHandshake::default()
    };

    let decoded = decode_extension_handshake(&encode_extension_handshake(&handshake)).unwrap();

    assert_eq!(decoded.client.as_deref(), Some("Styx"));
}

#[test]
fn extension_handshake_rejects_non_utf8_client_string() {
    let err = decode_extension_handshake(b"d1:mde1:v1:\xffe").unwrap_err();

    assert!(matches!(err, ExtensionError::InvalidUtf8 { field: "v" }));
}

#[test]
fn extension_handshake_rejects_non_utf8_extension_name() {
    let err = decode_extension_handshake(b"d1:md1:\xffi1eee").unwrap_err();

    assert!(matches!(err, ExtensionError::InvalidUtf8 { field: "m" }));
}

#[test]
fn extension_handshake_encodes_ipv4_using_yourip() {
    let handshake = ExtensionHandshake {
        ipv4: Some(Ipv4Addr::new(10, 20, 30, 40)),
        ..ExtensionHandshake::default()
    };

    let encoded = encode_extension_handshake(&handshake);

    assert!(encoded.windows(4).any(|window| window == [10, 20, 30, 40]));
}

#[test]
fn extension_handshake_decodes_raw_bytes_without_copying_semantics_assumptions() {
    let decoded =
        decode_extension_handshake(b"d4:ipv616:\0\0\0\0\0\0\0\0\0\0\xff\xff\x7f\0\0\x011:mdee")
            .unwrap();

    assert_eq!(
        decoded.ipv6,
        Some(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x7f00, 0x0001))
    );
}

#[test]
fn extension_handshake_can_encode_empty_handshake() {
    let encoded = encode_extension_handshake(&ExtensionHandshake::default());

    assert_eq!(Bytes::from(encoded), Bytes::from_static(b"d1:mdee"));
}
