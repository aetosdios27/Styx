use std::net::{Ipv4Addr, SocketAddr};

use styx_proto::InfoHashV1;
use styx_runtime::{decode_lsd_announce, encode_lsd_announce, LsdAnnounce};

#[test]
fn lsd_announce_encodes_single_info_hash() {
    let announce = LsdAnnounce {
        info_hashes: vec![InfoHashV1::new([0xab; 20])],
        port: 6881,
        cookie: "local-cookie".to_owned(),
    };

    let packet = encode_lsd_announce(&announce).unwrap();
    let text = std::str::from_utf8(&packet).unwrap();

    assert!(text.starts_with("BT-SEARCH * HTTP/1.1\r\n"));
    assert!(text.contains("Port: 6881\r\n"));
    assert!(text.contains("Infohash: ABABABABABABABABABABABABABABABABABABABAB\r\n"));
    assert!(packet.len() < 1400);
}

#[test]
fn lsd_announce_encodes_multiple_hashes_under_packet_cap() {
    let announce = LsdAnnounce {
        info_hashes: (0..20).map(|byte| InfoHashV1::new([byte; 20])).collect(),
        port: 6881,
        cookie: "cookie".to_owned(),
    };

    let packet = encode_lsd_announce(&announce).unwrap();

    assert!(packet.len() <= 1400);
}

#[test]
fn lsd_decode_returns_peer_from_udp_source_and_port_header() {
    let source = SocketAddr::from((Ipv4Addr::new(192, 168, 1, 20), 51413));
    let packet = b"BT-SEARCH * HTTP/1.1\r\nHost: 239.192.152.143:6771\r\nPort: 6889\r\nInfohash: ABABABABABABABABABABABABABABABABABABABAB\r\nCookie: remote\r\n\r\n";

    let decoded = decode_lsd_announce(packet, source, "local").unwrap();

    assert_eq!(decoded.peer, SocketAddr::from((source.ip(), 6889)));
    assert_eq!(decoded.info_hashes, vec![InfoHashV1::new([0xab; 20])]);
}

#[test]
fn lsd_ignores_own_cookie() {
    let source = SocketAddr::from((Ipv4Addr::LOCALHOST, 51413));
    let packet = b"BT-SEARCH * HTTP/1.1\r\nPort: 6889\r\nInfohash: ABABABABABABABABABABABABABABABABABABABAB\r\nCookie: same\r\n\r\n";

    let decoded = decode_lsd_announce(packet, source, "same").unwrap();

    assert!(decoded.info_hashes.is_empty());
}
