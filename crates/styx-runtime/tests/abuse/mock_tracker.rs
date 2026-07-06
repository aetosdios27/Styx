#![allow(dead_code)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use styx_proto::{encode, BencodeValue};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub struct MockTracker {
    behavior: MockTrackerBehavior,
}

pub enum MockTrackerBehavior {
    Normal(Vec<SocketAddr>),
    Http500,
    MalformedBencode,
    EmptyPeerList,
    NeverResponds,
    Delayed(u64),
}

impl MockTracker {
    pub fn new(behavior: MockTrackerBehavior) -> Self {
        Self { behavior }
    }

    pub async fn serve(self) -> (SocketAddr, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                match self.behavior {
                    MockTrackerBehavior::NeverResponds => {
                        // Accept the connection and drop it immediately.
                        // The HTTP client sees a clean TCP close before any
                        // response bytes arrive, which triggers an IO error
                        // or timeout on its side.
                    }
                    _ => {
                        let mut buf = [0u8; 4096];
                        let _n = stream.read(&mut buf).await.unwrap_or(0);

                        let response = match &self.behavior {
                            MockTrackerBehavior::Http500 => {
                                b"HTTP/1.1 500 Internal Server Error\r\n\r\n".to_vec()
                            }
                            MockTrackerBehavior::MalformedBencode => {
                                http_ok(b"this is not valid bencode")
                            }
                            MockTrackerBehavior::EmptyPeerList => http_ok(&announce_response(&[])),
                            MockTrackerBehavior::Delayed(ms) => {
                                tokio::time::sleep(Duration::from_millis(*ms)).await;
                                http_ok(&announce_response(&[]))
                            }
                            MockTrackerBehavior::Normal(peers) => {
                                http_ok(&announce_response(peers))
                            }
                            MockTrackerBehavior::NeverResponds => unreachable!(),
                        };

                        let _ = stream.write_all(&response).await;
                    }
                }
            }
        });
        (addr, handle)
    }
}

fn announce_response(peers: &[SocketAddr]) -> Vec<u8> {
    let compact = compact_peers(peers);
    let mut dict = BTreeMap::new();
    dict.insert(b"complete".to_vec(), BencodeValue::Integer(0));
    dict.insert(b"incomplete".to_vec(), BencodeValue::Integer(0));
    dict.insert(b"interval".to_vec(), BencodeValue::Integer(1800));
    dict.insert(b"peers".to_vec(), BencodeValue::Bytes(Bytes::from(compact)));
    encode(&BencodeValue::Dict(dict))
}

fn http_ok(body: &[u8]) -> Vec<u8> {
    let mut response = Vec::new();
    response.extend_from_slice(b"HTTP/1.1 200 OK\r\n");
    response.extend_from_slice(b"Content-Type: text/plain\r\n");
    response.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    response.extend_from_slice(b"\r\n");
    response.extend_from_slice(body);
    response
}

fn compact_peers(peers: &[SocketAddr]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(peers.len() * 6);
    for peer in peers {
        if let SocketAddr::V4(v4) = peer {
            buf.extend_from_slice(&v4.ip().octets());
            buf.extend_from_slice(&v4.port().to_be_bytes());
        }
    }
    buf
}
