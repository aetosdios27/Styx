use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Instant;

use bytes::Bytes;
use styx_dht::{
    DhtConfig, DhtMessage, DhtQuery, DhtResponse, DhtRuntime, DhtSocket, DhtSocketRuntime,
    NodeAddr, NodeId, TokenManager, TransactionId,
};

#[test]
fn runtime_bootstrap_queries_configured_nodes_with_ping() {
    let local_id = NodeId::new([1; 20]);
    let mut config = DhtConfig::default();
    let bootstrap_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 40)), 6881);
    config.add_bootstrap_node(bootstrap_addr);
    let mut runtime = runtime(local_id, config);

    let outbound = runtime.start_bootstrap(Instant::now()).unwrap();

    assert_eq!(outbound.len(), 1);
    let DhtMessage::Query {
        query: DhtQuery::Ping { id },
        ..
    } = outbound[0].1.clone()
    else {
        panic!("expected bootstrap ping");
    };
    assert_eq!(
        (outbound[0].0, id),
        (NodeAddr::new(bootstrap_addr), local_id)
    );
}

#[tokio::test]
#[ignore = "requires UDP socket permissions in the test environment"]
async fn socket_runtime_bootstraps_from_local_responder() {
    let bootstrap_socket = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let client_socket = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let bootstrap_addr = bootstrap_socket.local_addr().unwrap();
    let bootstrap_id = NodeId::new([44; 20]);
    let mut config = DhtConfig::default();
    config.add_bootstrap_node(bootstrap_addr);
    let mut client_runtime =
        DhtSocketRuntime::new(client_socket, runtime(NodeId::new([45; 20]), config));
    let bootstrap = tokio::spawn(async move {
        let request = bootstrap_socket.poll_once().await.unwrap();
        let DhtMessage::Query { transaction_id, .. } = request.message else {
            panic!("expected bootstrap query");
        };
        bootstrap_socket
            .send_to(
                &DhtMessage::Response {
                    transaction_id,
                    response: DhtResponse::Ping { id: bootstrap_id },
                },
                request.source,
            )
            .await
            .unwrap();
    });

    client_runtime.bootstrap_once().await.unwrap();
    bootstrap.await.unwrap();

    assert_eq!(client_runtime.runtime().node().routing().len(), 1);
}

fn runtime(id: NodeId, config: DhtConfig) -> DhtRuntime {
    DhtRuntime::new(
        id,
        TokenManager::with_secrets(
            Bytes::from_static(b"bootstrap-current"),
            Bytes::from_static(b"bootstrap-previous"),
        ),
        config,
    )
    .unwrap()
}

#[allow(dead_code)]
fn tx(value: u8) -> TransactionId {
    TransactionId::new(vec![value]).unwrap()
}
