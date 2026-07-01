use std::net::Ipv4Addr;

use styx_dht::{generate_bep42_ipv4_id, is_bep42_ipv4_id};

#[test]
fn bep42_validation_accepts_official_ipv4_test_vectors() {
    let vectors = [
        (
            Ipv4Addr::new(124, 31, 75, 21),
            "5fbfbff10c5d6a4ec8a88e4c6ab4c28b95eee401",
        ),
        (
            Ipv4Addr::new(21, 75, 31, 124),
            "5a3ce9c14e7a08645677bbd1cfe7d8f956d53256",
        ),
        (
            Ipv4Addr::new(65, 23, 51, 170),
            "a5d43220bc8f112a3d426c84764f8c2a1150e616",
        ),
        (
            Ipv4Addr::new(84, 124, 73, 14),
            "1b0321dd1bb1fe518101ceef99462b947a01ff41",
        ),
        (
            Ipv4Addr::new(43, 213, 53, 83),
            "e56f6cbf5b7c4be0237986d5243b87aa6d51305a",
        ),
    ];

    for (ip, node_id) in vectors {
        assert!(is_bep42_ipv4_id(ip, &hex20(node_id)));
    }
}

#[test]
fn bep42_validation_rejects_wrong_prefix() {
    let ip = Ipv4Addr::new(124, 31, 75, 21);
    let mut id = hex20("5fbfbff10c5d6a4ec8a88e4c6ab4c28b95eee401");
    id[0] ^= 0x80;

    assert!(!is_bep42_ipv4_id(ip, &id));
}

#[test]
fn bep42_generation_produces_valid_id_for_ipv4() {
    let ip = Ipv4Addr::new(8, 8, 8, 8);
    let id = generate_bep42_ipv4_id(ip, 0x42, [0xab; 16]);

    assert!(is_bep42_ipv4_id(ip, id.as_bytes()));
    assert_eq!(id.as_bytes()[19], 0x42);
}

fn hex20(input: &str) -> [u8; 20] {
    let mut output = [0_u8; 20];
    for (index, chunk) in input.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(chunk).unwrap();
        output[index] = u8::from_str_radix(text, 16).unwrap();
    }
    output
}
