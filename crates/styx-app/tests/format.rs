use std::str::FromStr;

use styx_app::format::{format_bytes, format_percent, format_rate, sparkline, InfoHashHex};

#[test]
fn info_hash_hex_accepts_forty_hex_characters() {
    let hash = InfoHashHex::from_str("0123456789abcdef0123456789ABCDEF01234567").unwrap();

    assert_eq!(hash.to_string(), "0123456789abcdef0123456789abcdef01234567");
}

#[test]
fn info_hash_hex_rejects_wrong_length() {
    let err = InfoHashHex::from_str("abcd").unwrap_err();

    assert!(err.to_string().contains("40 hex characters"));
}

#[test]
fn byte_formatting_uses_binary_units() {
    assert_eq!(format_bytes(1_536), "1.5 KiB");
    assert_eq!(format_bytes(1_048_576), "1.0 MiB");
}

#[test]
fn rate_formatting_appends_per_second() {
    assert_eq!(format_rate(2_097_152), "2.0 MiB/s");
}

#[test]
fn percent_formatting_clamps_to_display_range() {
    assert_eq!(format_percent(1.25), "100.0%");
    assert_eq!(format_percent(f32::NAN), "0.0%");
}

#[test]
fn sparkline_is_width_stable_for_empty_input() {
    assert_eq!(sparkline(&[], 5).chars().count(), 5);
}
