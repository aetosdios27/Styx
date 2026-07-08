use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use styx_proto::{parse_magnet_uri, MagnetError};

#[test]
fn magnet_parser_accepts_v1_btih_hex() {
    let parsed =
        parse_magnet_uri("magnet:?xt=urn:btih:631a31dd0a46257d5078c0dee4e66e26f73e42ac").unwrap();

    assert_eq!(
        parsed.info_hash_v1.unwrap().as_bytes(),
        &[
            0x63, 0x1a, 0x31, 0xdd, 0x0a, 0x46, 0x25, 0x7d, 0x50, 0x78, 0xc0, 0xde, 0xe4, 0xe6,
            0x6e, 0x26, 0xf7, 0x3e, 0x42, 0xac,
        ]
    );
}

#[test]
fn magnet_parser_accepts_v1_btih_base32() {
    let parsed = parse_magnet_uri("magnet:?xt=urn:btih:MMNDDXIKIYSX2UDYYDPOJZTOE33T4QVM").unwrap();

    assert_eq!(
        parsed.info_hash_v1.unwrap().as_bytes(),
        &[
            0x63, 0x1a, 0x31, 0xdd, 0x0a, 0x46, 0x25, 0x7d, 0x50, 0x78, 0xc0, 0xde, 0xe4, 0xe6,
            0x6e, 0x26, 0xf7, 0x3e, 0x42, 0xac,
        ]
    );
}

#[test]
fn magnet_parser_accepts_v2_btmh_multihash() {
    let parsed = parse_magnet_uri(
        "magnet:?xt=urn:btmh:1220d8dd32ac93357c368556af3ac1d95c9d76bd0dff6fa9833ecdac3d53134efabb",
    )
    .unwrap();

    assert_eq!(
        parsed.info_hash_v2.unwrap().to_string(),
        "d8dd32ac93357c368556af3ac1d95c9d76bd0dff6fa9833ecdac3d53134efabb"
    );
}

#[test]
fn magnet_parser_accepts_hybrid_magnet_with_trackers_and_display_name() {
    let parsed = parse_magnet_uri(
        "magnet:?xt=urn:btih:631a31dd0a46257d5078c0dee4e66e26f73e42ac&xt=urn:btmh:1220d8dd32ac93357c368556af3ac1d95c9d76bd0dff6fa9833ecdac3d53134efabb&dn=Arch%20Linux&tr=https%3A%2F%2Ftracker.example%2Fannounce&tr=udp%3A%2F%2Ftracker.example%3A6969",
    )
    .unwrap();

    assert!(parsed.info_hash_v1.is_some());
    assert!(parsed.info_hash_v2.is_some());
    assert_eq!(parsed.display_name.as_deref(), Some("Arch Linux"));
    assert_eq!(parsed.trackers.len(), 2);
}

#[test]
fn magnet_parser_accepts_exact_ipv4_and_ipv6_peers() {
    let parsed = parse_magnet_uri(
        "magnet:?xt=urn:btih:631a31dd0a46257d5078c0dee4e66e26f73e42ac&x.pe=127.0.0.1%3A6881&x.pe=%5B%3A%3A1%5D%3A6882",
    )
    .unwrap();

    assert_eq!(
        parsed.exact_peers,
        vec![
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 6881),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 6882)
        ]
    );
}

#[test]
fn magnet_parser_accepts_bep53_select_only_indices_and_ranges() {
    let parsed =
        parse_magnet_uri("magnet:?xt=urn:btih:631a31dd0a46257d5078c0dee4e66e26f73e42ac&so=0,2,4-6")
            .unwrap();

    assert_eq!(parsed.select_only, Some(vec![0, 2, 4, 5, 6]));
}

#[test]
fn magnet_parser_requires_magnet_scheme() {
    let err = parse_magnet_uri("https://example.invalid").unwrap_err();

    assert!(matches!(err, MagnetError::InvalidScheme));
}

#[test]
fn magnet_parser_requires_at_least_one_supported_xt() {
    let err = parse_magnet_uri("magnet:?dn=nohash").unwrap_err();

    assert!(matches!(err, MagnetError::MissingXt));
}

#[test]
fn magnet_parser_rejects_invalid_btih_length() {
    let err = parse_magnet_uri("magnet:?xt=urn:btih:abcd").unwrap_err();

    assert!(matches!(err, MagnetError::InvalidHashLength));
}

#[test]
fn magnet_parser_rejects_invalid_tracker_url() {
    let err = parse_magnet_uri(
        "magnet:?xt=urn:btih:631a31dd0a46257d5078c0dee4e66e26f73e42ac&tr=not-a-url",
    )
    .unwrap_err();

    assert!(matches!(err, MagnetError::InvalidTrackerUrl { .. }));
}

#[test]
fn magnet_parser_rejects_invalid_exact_peer() {
    let err = parse_magnet_uri(
        "magnet:?xt=urn:btih:631a31dd0a46257d5078c0dee4e66e26f73e42ac&x.pe=127.0.0.1",
    )
    .unwrap_err();

    assert!(matches!(err, MagnetError::InvalidExactPeer { .. }));
}

#[test]
fn magnet_parser_rejects_invalid_select_only_range() {
    let err =
        parse_magnet_uri("magnet:?xt=urn:btih:631a31dd0a46257d5078c0dee4e66e26f73e42ac&so=9-3")
            .unwrap_err();

    assert!(matches!(err, MagnetError::InvalidSelectOnly { .. }));
}
