use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use styx_dht::{DhtMessage, DhtQuery, DhtSocket, NodeId, TransactionId};

#[tokio::test]
#[ignore = "requires UDP socket permissions in the test environment"]
async fn udp_socket_exchanges_krpc_message() {
    let first = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let second = DhtSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let message = DhtMessage::Query {
        transaction_id: TransactionId::new(vec![b'a']).unwrap(),
        query: DhtQuery::Ping {
            id: NodeId::new([1; 20]),
        },
    };

    first
        .send_to(&message, second.local_addr().unwrap())
        .await
        .unwrap();
    let event = second.poll_once().await.unwrap();

    assert_eq!(event.message, message);
    assert_eq!(event.source, first.local_addr().unwrap());
}
