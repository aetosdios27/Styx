use std::time::Duration;
use styx_proto::{
    encode_message, read_handshake, read_message, write_handshake, write_message, Handshake,
    PeerMessage, DEFAULT_MAX_PEER_FRAME_LEN,
};
use tokio::io::{AsyncRead, AsyncWrite};

/// Scripted event for a [`MockPeer`].
pub enum ScriptedEvent {
    /// Encode and send a protocol message.
    Send(PeerMessage),
    /// Write raw bytes to the wire (protocol-violating garbage).
    SendRaw(Vec<u8>),
    /// Read and assert the next inbound message matches the expectation within a timeout.
    Receive {
        expect: PeerMessage,
        timeout: Duration,
    },
    /// Shut down the write half (simulate connection drop).
    Drop,
    /// Send no data for the given duration.
    Stall(Duration),
    /// Apply bit-flip corruption to the *next* sent message. `(num_bits, byte_offset)`
    CorruptNext(u8, usize),
}

/// Builder for a scripted mock peer.
pub struct MockPeerBuilder {
    handshake: Handshake,
    events: Vec<ScriptedEvent>,
}

impl MockPeerBuilder {
    /// Create a new builder with the handshake the mock will respond with.
    /// The mock will validate the incoming handshake against `handshake.info_hash`.
    pub fn new(handshake: Handshake) -> Self {
        Self {
            handshake,
            events: Vec::new(),
        }
    }

    /// Append a scripted event.
    pub fn add_event(mut self, event: ScriptedEvent) -> Self {
        self.events.push(event);
        self
    }

    /// Consume the builder and produce a [`MockPeer`].
    pub fn build(self) -> MockPeer {
        MockPeer {
            handshake: self.handshake,
            events: self.events,
        }
    }
}

/// A scripted mock peer that runs a sequence of events against a stream.
pub struct MockPeer {
    handshake: Handshake,
    events: Vec<ScriptedEvent>,
}

impl MockPeer {
    /// Execute the script, consuming the mock.
    ///
    /// 1. Read the incoming handshake and validate it against `info_hash`.
    /// 2. Respond with the configured handshake.
    /// 3. Execute each scripted event in order.
    pub async fn run(self, mut stream: impl AsyncRead + AsyncWrite + Unpin) {
        let _incoming = read_handshake(&mut stream, self.handshake.info_hash)
            .await
            .expect("MockPeer: incoming handshake failed or info_hash mismatch");

        write_handshake(&mut stream, &self.handshake)
            .await
            .expect("MockPeer: failed to write handshake");

        let mut pending_corrupt: Option<(u8, usize)> = None;

        for event in self.events {
            match event {
                ScriptedEvent::Send(msg) => {
                    let mut encoded = encode_message(&msg)
                        .expect("MockPeer: failed to encode message")
                        .to_vec();
                    if let Some((bits, off)) = pending_corrupt.take() {
                        apply_corruption(&mut encoded, bits, off);
                    }
                    use tokio::io::AsyncWriteExt;
                    stream
                        .write_all(&encoded)
                        .await
                        .expect("MockPeer: failed to write message");
                }
                ScriptedEvent::SendRaw(mut bytes) => {
                    if let Some((bits, off)) = pending_corrupt.take() {
                        apply_corruption(&mut bytes, bits, off);
                    }
                    use tokio::io::AsyncWriteExt;
                    stream
                        .write_all(&bytes)
                        .await
                        .expect("MockPeer: failed to write raw bytes");
                }
                ScriptedEvent::Receive { expect, timeout } => {
                    let result = tokio::time::timeout(
                        timeout,
                        read_message(&mut stream, DEFAULT_MAX_PEER_FRAME_LEN),
                    )
                    .await
                    .expect("MockPeer: timed out waiting for message")
                    .expect("MockPeer: failed to read message");
                    assert_eq!(result, expect, "MockPeer: received message mismatch");
                }
                ScriptedEvent::Drop => {
                    use tokio::io::AsyncWriteExt;
                    let _ = stream.shutdown().await;
                    break;
                }
                ScriptedEvent::Stall(duration) => {
                    tokio::time::sleep(duration).await;
                }
                ScriptedEvent::CorruptNext(num_bits, offset) => {
                    pending_corrupt = Some((num_bits, offset));
                }
            }
        }
    }
}

/// Flip the least-significant bit of `num_bits` consecutive bytes starting at `offset`.
fn apply_corruption(bytes: &mut [u8], num_bits: u8, offset: usize) {
    for i in 0..usize::from(num_bits) {
        let pos = offset + i;
        if pos < bytes.len() {
            bytes[pos] ^= 0x01;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use styx_proto::{ExtensionBits, InfoHashV1, PeerId};
    use tokio::io::duplex;

    fn info_hash(byte: u8) -> InfoHashV1 {
        InfoHashV1::new([byte; 20])
    }

    fn peer_id(byte: u8) -> PeerId {
        PeerId::new([byte; 20])
    }

    fn test_handshake() -> Handshake {
        Handshake {
            reserved: ExtensionBits::default(),
            info_hash: info_hash(1),
            peer_id: peer_id(2),
        }
    }

    #[tokio::test]
    async fn handshake_and_message_exchange() {
        let (mut client, server) = duplex(4096);
        let handshake = test_handshake();

        let mock = MockPeerBuilder::new(handshake)
            .add_event(ScriptedEvent::Receive {
                expect: PeerMessage::Interested,
                timeout: Duration::from_secs(5),
            })
            .add_event(ScriptedEvent::Send(PeerMessage::Unchoke))
            .build();

        let peer_handle = tokio::spawn(async move { mock.run(server).await });

        write_handshake(&mut client, &handshake).await.unwrap();
        let _incoming = read_handshake(&mut client, handshake.info_hash)
            .await
            .unwrap();

        write_message(&mut client, &PeerMessage::Interested)
            .await
            .unwrap();

        assert_eq!(
            read_message(&mut client, DEFAULT_MAX_PEER_FRAME_LEN)
                .await
                .unwrap(),
            PeerMessage::Unchoke
        );

        peer_handle.await.unwrap();
    }

    #[tokio::test]
    async fn drop_event_closes_connection() {
        let (mut client, server) = duplex(4096);
        let handshake = test_handshake();

        let mock = MockPeerBuilder::new(handshake)
            .add_event(ScriptedEvent::Drop)
            .build();

        let peer_handle = tokio::spawn(async move { mock.run(server).await });

        write_handshake(&mut client, &handshake).await.unwrap();
        let _incoming = read_handshake(&mut client, handshake.info_hash)
            .await
            .unwrap();

        let result = read_message(&mut client, DEFAULT_MAX_PEER_FRAME_LEN).await;
        assert!(result.is_err(), "client should see EOF after drop");

        peer_handle.await.unwrap();
    }

    #[tokio::test]
    async fn stall_delays_then_continues() {
        let (mut client, server) = duplex(4096);
        let handshake = test_handshake();

        let mock = MockPeerBuilder::new(handshake)
            .add_event(ScriptedEvent::Stall(Duration::from_millis(50)))
            .add_event(ScriptedEvent::Send(PeerMessage::Unchoke))
            .build();

        let peer_handle = tokio::spawn(async move { mock.run(server).await });

        write_handshake(&mut client, &handshake).await.unwrap();
        let _incoming = read_handshake(&mut client, handshake.info_hash)
            .await
            .unwrap();

        assert_eq!(
            read_message(&mut client, DEFAULT_MAX_PEER_FRAME_LEN)
                .await
                .unwrap(),
            PeerMessage::Unchoke
        );

        peer_handle.await.unwrap();
    }

    #[tokio::test]
    async fn corrupt_next_flips_bits_in_outbound_message() {
        let (mut client, server) = duplex(4096);
        let handshake = test_handshake();

        let mock = MockPeerBuilder::new(handshake)
            .add_event(ScriptedEvent::CorruptNext(1, 0))
            .add_event(ScriptedEvent::Send(PeerMessage::Unchoke))
            .build();

        let peer_handle = tokio::spawn(async move { mock.run(server).await });

        write_handshake(&mut client, &handshake).await.unwrap();
        let _incoming = read_handshake(&mut client, handshake.info_hash)
            .await
            .unwrap();

        let result = read_message(&mut client, DEFAULT_MAX_PEER_FRAME_LEN).await;
        // Corrupting byte 0 (MSB of length prefix) changes [0,0,0,1,1] length to 16777217,
        // which exceeds the frame cap.
        assert!(
            matches!(result, Err(styx_proto::PeerWireError::FrameTooLarge { .. })),
            "expected FrameTooLarge, got {result:?}"
        );
        peer_handle.await.unwrap();
    }

    #[tokio::test]
    async fn send_raw_writes_bytes_directly() {
        let (mut client, server) = duplex(4096);
        let handshake = test_handshake();
        let garbage = vec![0xff, 0xfe, 0xfd];

        let mock = MockPeerBuilder::new(handshake)
            .add_event(ScriptedEvent::SendRaw(garbage.clone()))
            .build();

        let peer_handle = tokio::spawn(async move { mock.run(server).await });

        write_handshake(&mut client, &handshake).await.unwrap();
        let _incoming = read_handshake(&mut client, handshake.info_hash)
            .await
            .unwrap();

        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 3];
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, garbage);

        peer_handle.await.unwrap();
    }
}
