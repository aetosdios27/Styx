use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use styx_proto::{decode_pex_message, encode_pex_message, PexMessage};

#[test]
fn pex_message_decodes_ipv4_and_ipv6_contacts() {
    let message = PexMessage {
        added: vec![SocketAddr::from((Ipv4Addr::new(203, 0, 113, 7), 6881))],
        added6: vec![SocketAddr::from((Ipv6Addr::LOCALHOST, 6882))],
        dropped: vec![SocketAddr::from((Ipv4Addr::new(198, 51, 100, 9), 6883))],
        dropped6: Vec::new(),
        added_flags: vec![0x05],
        added6_flags: vec![0x02],
    };

    let encoded = encode_pex_message(&message).unwrap();
    let decoded = decode_pex_message(&encoded).unwrap();

    assert_eq!(decoded, message);
}

#[test]
fn pex_message_rejects_invalid_compact_lengths() {
    let err = decode_pex_message(b"d5:added5:abcdee").unwrap_err();

    assert!(err.to_string().contains("multiple of 6"));
}

#[test]
fn pex_message_rejects_more_than_fifty_contacts_per_family() {
    let message = PexMessage {
        added: (1..=51)
            .map(|port| SocketAddr::from((Ipv4Addr::new(203, 0, 113, 7), port)))
            .collect(),
        ..PexMessage::default()
    };

    let err = encode_pex_message(&message).unwrap_err();

    assert!(err.to_string().contains("50"));
}
