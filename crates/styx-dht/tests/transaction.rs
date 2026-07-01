use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use styx_dht::{
    DhtError, InfoHash, NodeAddr, NodeId, TransactionId, TransactionKind, TransactionTable,
};

#[test]
fn transaction_table_matches_response_from_same_source_and_transaction_id() {
    let mut table = TransactionTable::with_capacity(8);
    let target = addr(7);
    let now = Instant::now();
    let id = TransactionId::new(vec![1, 2]).unwrap();

    table
        .insert(
            id.clone(),
            target,
            TransactionKind::GetPeers {
                info_hash: InfoHash::new([9; 20]),
            },
            now,
            Duration::from_secs(5),
        )
        .unwrap();

    let matched = table.match_response(&id, target, now + Duration::from_secs(1));

    assert_eq!(
        matched.unwrap().kind,
        TransactionKind::GetPeers {
            info_hash: InfoHash::new([9; 20])
        }
    );
}

#[test]
fn transaction_table_rejects_response_from_wrong_source() {
    let mut table = TransactionTable::with_capacity(8);
    let now = Instant::now();
    let id = TransactionId::new(vec![3]).unwrap();

    table
        .insert(
            id.clone(),
            addr(1),
            TransactionKind::Ping,
            now,
            Duration::from_secs(5),
        )
        .unwrap();

    let matched = table.match_response(&id, addr(2), now + Duration::from_secs(1));

    assert_eq!(matched.unwrap_err(), DhtError::UnexpectedTransaction);
}

#[test]
fn transaction_table_drains_expired_transactions() {
    let mut table = TransactionTable::with_capacity(8);
    let now = Instant::now();

    table
        .insert(
            TransactionId::new(vec![4]).unwrap(),
            addr(1),
            TransactionKind::FindNode {
                target: NodeId::new([8; 20]),
            },
            now,
            Duration::from_secs(5),
        )
        .unwrap();

    let expired = table.drain_expired(now + Duration::from_secs(6));

    assert_eq!(expired.len(), 1);
}

#[test]
fn transaction_table_rejects_insert_when_capacity_is_exhausted() {
    let mut table = TransactionTable::with_capacity(1);
    let now = Instant::now();

    table
        .insert(
            TransactionId::new(vec![1]).unwrap(),
            addr(1),
            TransactionKind::Ping,
            now,
            Duration::from_secs(5),
        )
        .unwrap();

    let result = table.insert(
        TransactionId::new(vec![2]).unwrap(),
        addr(2),
        TransactionKind::Ping,
        now,
        Duration::from_secs(5),
    );

    assert_eq!(result.unwrap_err(), DhtError::TransactionTableFull);
}

fn addr(last_octet: u8) -> NodeAddr {
    NodeAddr::new(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(198, 51, 100, last_octet)),
        6881,
    ))
}
