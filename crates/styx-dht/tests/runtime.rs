use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Instant;

use bytes::Bytes;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use styx_dht::{
    is_bep42_ipv6_id, CompactNode, CompactPeer, DhtConfig, DhtEvent, DhtIdentityAction, DhtMessage,
    DhtQuery, DhtResponse, DhtRuntime, ExternalIp, InfoHash, NodeAddr, NodeId, RuntimeAction,
    TokenManager, TransactionKind,
};

#[test]
fn runtime_registers_outbound_query_and_matches_response() {
    let local_id = NodeId::new([1; 20]);
    let remote_id = NodeId::new([2; 20]);
    let mut runtime = runtime(local_id);
    let now = Instant::now();
    let remote = addr(2);

    let query = runtime
        .start_query(remote, DhtQuery::Ping { id: local_id }, now)
        .unwrap();
    let DhtMessage::Query { transaction_id, .. } = query else {
        panic!("expected query message");
    };
    let response = DhtMessage::Response {
        transaction_id,
        response: DhtResponse::Ping { id: remote_id },
    };

    let action = runtime.handle_message(response, remote, now).unwrap();

    assert_eq!(
        action.event,
        Some(DhtEvent::ResponseMatched {
            source: remote,
            kind: TransactionKind::Ping
        })
    );
}

#[test]
fn runtime_dispatches_inbound_query_to_node_handler() {
    let local_id = NodeId::new([3; 20]);
    let remote_id = NodeId::new([4; 20]);
    let mut runtime = runtime(local_id);
    let source = addr(4);
    let message = DhtMessage::Query {
        transaction_id: tx(9),
        query: DhtQuery::Ping { id: remote_id },
    };

    let action = runtime
        .handle_message(message, source, Instant::now())
        .unwrap();

    assert_eq!(
        action,
        RuntimeAction {
            response: Some(DhtMessage::Response {
                transaction_id: tx(9),
                response: DhtResponse::Ping { id: local_id },
            }),
            event: Some(DhtEvent::QueryResponded { target: source }),
            outbound: Vec::new(),
        }
    );
}

#[test]
fn runtime_reports_unsolicited_response_without_mutating_transactions() {
    let mut runtime = runtime(NodeId::new([5; 20]));
    let source = addr(5);
    let message = DhtMessage::Response {
        transaction_id: tx(1),
        response: DhtResponse::Ping {
            id: NodeId::new([6; 20]),
        },
    };

    let action = runtime
        .handle_message(message, source, Instant::now())
        .unwrap();

    assert_eq!(action.event, Some(DhtEvent::UnsolicitedResponse { source }));
}

#[test]
fn runtime_get_peers_starts_queries_to_closest_routing_nodes() {
    let local_id = NodeId::new([7; 20]);
    let mut runtime = runtime(local_id);
    let first = CompactNode {
        id: NodeId::new([1; 20]),
        addr: addr(1),
    };
    let second = CompactNode {
        id: NodeId::new([2; 20]),
        addr: addr(2),
    };
    runtime
        .node_mut()
        .routing_mut()
        .insert(first.id, first.addr)
        .unwrap();
    runtime
        .node_mut()
        .routing_mut()
        .insert(second.id, second.addr)
        .unwrap();

    let outbound = runtime
        .start_get_peers(InfoHash::new([9; 20]), Instant::now())
        .unwrap();

    assert_eq!(outbound.len(), 2);
}

#[test]
fn runtime_get_peers_response_emits_peers_and_retains_token() {
    let local_id = NodeId::new([8; 20]);
    let remote_id = NodeId::new([9; 20]);
    let mut runtime = runtime(local_id);
    let remote = addr(9);
    runtime
        .node_mut()
        .routing_mut()
        .insert(remote_id, remote)
        .unwrap();
    let info_hash = InfoHash::new([4; 20]);
    let outbound = runtime.start_get_peers(info_hash, Instant::now()).unwrap();
    let DhtMessage::Query { transaction_id, .. } = outbound[0].1.clone() else {
        panic!("expected get_peers query");
    };
    let peer = CompactPeer::new(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 55)),
        51413,
    ));
    let token = Bytes::from_static(b"announce-token");

    let action = runtime
        .handle_message(
            DhtMessage::Response {
                transaction_id,
                response: DhtResponse::GetPeers {
                    id: remote_id,
                    token: token.clone(),
                    values: vec![peer],
                    nodes: Vec::new(),
                    nodes6: Vec::new(),
                    external_ip: None,
                },
            },
            remote,
            Instant::now(),
        )
        .unwrap();

    assert_eq!(
        action.event,
        Some(DhtEvent::PeersDiscovered {
            info_hash,
            peers: vec![peer],
        })
    );
    assert_eq!(runtime.token_for(info_hash, remote), Some(token));
}

#[test]
fn runtime_announce_peer_uses_token_from_get_peers_responder() {
    let local_id = NodeId::new([10; 20]);
    let remote_id = NodeId::new([11; 20]);
    let mut runtime = runtime(local_id);
    let remote = addr(11);
    runtime
        .node_mut()
        .routing_mut()
        .insert(remote_id, remote)
        .unwrap();
    let info_hash = InfoHash::new([5; 20]);
    let outbound = runtime.start_get_peers(info_hash, Instant::now()).unwrap();
    let DhtMessage::Query { transaction_id, .. } = outbound[0].1.clone() else {
        panic!("expected get_peers query");
    };
    let token = Bytes::from_static(b"announce-token");
    runtime
        .handle_message(
            DhtMessage::Response {
                transaction_id,
                response: DhtResponse::GetPeers {
                    id: remote_id,
                    token: token.clone(),
                    values: Vec::new(),
                    nodes: Vec::new(),
                    nodes6: Vec::new(),
                    external_ip: None,
                },
            },
            remote,
            Instant::now(),
        )
        .unwrap();

    let announces = runtime
        .start_announce_peer(info_hash, 51413, false, Instant::now())
        .unwrap();

    assert_eq!(announces.len(), 1);
    assert_eq!(announces[0].0, remote);
    let DhtMessage::Query {
        query:
            DhtQuery::AnnouncePeer {
                info_hash: actual_hash,
                port,
                token: actual_token,
                ..
            },
        ..
    } = announces[0].1.clone()
    else {
        panic!("expected announce_peer query");
    };
    assert_eq!((actual_hash, port, actual_token), (info_hash, 51413, token));
}

#[test]
fn runtime_expiring_transactions_marks_target_questionable() {
    let local_id = NodeId::new([12; 20]);
    let remote_id = NodeId::new([13; 20]);
    let mut runtime = runtime(local_id);
    let remote = addr(13);
    runtime
        .node_mut()
        .routing_mut()
        .insert(remote_id, remote)
        .unwrap();
    let now = Instant::now();
    runtime
        .start_query(remote, DhtQuery::Ping { id: local_id }, now)
        .unwrap();

    let expired = runtime
        .drain_timeouts(now + DhtConfig::default().query_timeout)
        .unwrap();

    assert_eq!(
        expired,
        vec![DhtEvent::TransactionExpired {
            target: remote,
            kind: TransactionKind::Ping,
        }]
    );
    assert_eq!(
        runtime.node().routing().node(remote_id).unwrap().status,
        styx_dht::NodeStatus::Questionable
    );
}

#[test]
fn runtime_surfaces_external_ip_observed_in_response() {
    let local_id = NodeId::new([14; 20]);
    let remote_id = NodeId::new([15; 20]);
    let mut runtime = runtime(local_id);
    let remote = addr(15);
    let now = Instant::now();
    let query = runtime
        .start_query(
            remote,
            DhtQuery::FindNode {
                id: local_id,
                target: remote_id,
                want: Vec::new(),
            },
            now,
        )
        .unwrap();
    let DhtMessage::Query { transaction_id, .. } = query else {
        panic!("expected query");
    };

    let action = runtime
        .handle_message(
            DhtMessage::Response {
                transaction_id,
                response: DhtResponse::FindNode {
                    id: remote_id,
                    nodes: Vec::new(),
                    nodes6: Vec::new(),
                    external_ip: Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 88))),
                },
            },
            remote,
            now,
        )
        .unwrap();

    assert_eq!(
        action.event,
        Some(DhtEvent::ExternalIpObserved {
            source: remote,
            ip: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 88)),
        })
    );
}

#[test]
fn runtime_external_ip_observation_requests_ipv6_bep42_restart() {
    let mut runtime = runtime(NodeId::new([1; 20]));
    let mut rng = ChaCha8Rng::seed_from_u64(44);
    let ip = Ipv6Addr::new(0x2001, 0x0db8, 0x0001, 0x0002, 0, 0, 0, 1);

    let action = runtime
        .observe_external_ip_for_identity(ExternalIp::V6(ip), &mut rng)
        .unwrap()
        .unwrap();

    assert!(matches!(
        action,
        DhtIdentityAction::RestartWithNodeId { identity }
            if is_bep42_ipv6_id(ip, identity.node_id.as_bytes())
    ));
}

#[test]
fn runtime_external_ip_observation_is_stable_after_valid_restart() {
    let mut runtime = runtime(NodeId::new([1; 20]));
    let mut rng = ChaCha8Rng::seed_from_u64(45);
    let ip = Ipv6Addr::new(0x2001, 0x0db8, 0x0001, 0x0002, 0, 0, 0, 2);

    runtime
        .observe_external_ip_for_identity(ExternalIp::V6(ip), &mut rng)
        .unwrap();
    let action = runtime
        .observe_external_ip_for_identity(ExternalIp::V6(ip), &mut rng)
        .unwrap();

    assert_eq!(action, None);
}

fn runtime(id: NodeId) -> DhtRuntime {
    DhtRuntime::new(
        id,
        TokenManager::with_secrets(
            Bytes::from_static(b"runtime-current"),
            Bytes::from_static(b"runtime-previous"),
        ),
        DhtConfig::default(),
    )
    .unwrap()
}

fn addr(last_octet: u8) -> NodeAddr {
    NodeAddr::new(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last_octet)),
        6881,
    ))
}

fn tx(value: u8) -> styx_dht::TransactionId {
    styx_dht::TransactionId::new(vec![value]).unwrap()
}
