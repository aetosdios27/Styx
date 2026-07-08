use bytes::Bytes;
use styx_proto::{
    decode_metadata_message, encode_metadata_message, metadata_piece_count, MetadataError,
    MetadataMessage,
};

#[test]
fn metadata_request_round_trips() {
    let message = MetadataMessage::Request { piece: 17 };

    let decoded = decode_metadata_message(&encode_metadata_message(&message)).unwrap();

    assert_eq!(decoded, message);
}

#[test]
fn metadata_data_splits_bencoded_header_from_payload() {
    let input = b"d8:msg_typei1e5:piecei2e10:total_sizei40000eedata";

    let decoded = decode_metadata_message(input).unwrap();

    assert_eq!(
        decoded,
        MetadataMessage::Data {
            piece: 2,
            total_size: 40_000,
            payload: Bytes::from_static(b"data")
        }
    );
}

#[test]
fn metadata_reject_round_trips() {
    let message = MetadataMessage::Reject { piece: 3 };

    let decoded = decode_metadata_message(&encode_metadata_message(&message)).unwrap();

    assert_eq!(decoded, message);
}

#[test]
fn metadata_decode_rejects_unknown_msg_type() {
    let err = decode_metadata_message(b"d8:msg_typei99e5:piecei0ee").unwrap_err();

    assert!(matches!(
        err,
        MetadataError::UnknownMessageType { msg_type: 99 }
    ));
}

#[test]
fn metadata_decode_rejects_data_without_payload() {
    let err = decode_metadata_message(b"d8:msg_typei1e5:piecei0e10:total_sizei1ee").unwrap_err();

    assert!(matches!(err, MetadataError::MissingPayload));
}

#[test]
fn metadata_piece_count_rounds_up_final_short_block() {
    let count = metadata_piece_count(16 * 1024 + 1).unwrap();

    assert_eq!(count, 2);
}

#[test]
fn metadata_piece_count_rejects_zero_total_size() {
    let err = metadata_piece_count(0).unwrap_err();

    assert!(matches!(
        err,
        MetadataError::InvalidTotalSize { total_size: 0 }
    ));
}

#[test]
fn metadata_decode_rejects_negative_piece() {
    let err = decode_metadata_message(b"d8:msg_typei0e5:piecei-1ee").unwrap_err();

    assert!(matches!(err, MetadataError::InvalidPiece { piece: -1 }));
}

#[test]
fn metadata_decode_rejects_negative_total_size() {
    let err = decode_metadata_message(b"d8:msg_typei1e5:piecei0e10:total_sizei-1eex").unwrap_err();

    assert!(matches!(
        err,
        MetadataError::InvalidTotalSizeValue { total_size: -1 }
    ));
}

#[test]
fn metadata_decode_rejects_missing_piece() {
    let err = decode_metadata_message(b"d8:msg_typei0ee").unwrap_err();

    assert!(matches!(
        err,
        MetadataError::MissingField { field: "piece" }
    ));
}

#[test]
fn metadata_decode_rejects_missing_msg_type() {
    let err = decode_metadata_message(b"d5:piecei0ee").unwrap_err();

    assert!(matches!(
        err,
        MetadataError::MissingField { field: "msg_type" }
    ));
}

#[test]
fn metadata_decode_rejects_trailing_bytes_for_request() {
    let err = decode_metadata_message(b"d8:msg_typei0e5:piecei0eex").unwrap_err();

    assert!(matches!(err, MetadataError::UnexpectedPayload { bytes: 1 }));
}

#[test]
fn metadata_data_round_trips() {
    let message = MetadataMessage::Data {
        piece: 5,
        total_size: 99_999,
        payload: Bytes::from_static(b"metadata bytes"),
    };

    let decoded = decode_metadata_message(&encode_metadata_message(&message)).unwrap();

    assert_eq!(decoded, message);
}
