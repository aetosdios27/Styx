use std::net::SocketAddr;
use std::time::Duration;

use styx_proto::{
    read_handshake, read_message, write_handshake, ExtensionBits, Handshake, InfoHashV1, PeerId,
    PeerMessage, PeerWireError, DEFAULT_MAX_PEER_FRAME_LEN,
};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::task::JoinHandle;
use tokio::time::timeout;

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum PeerIoCommand {
    Send(PeerMessage),
    Disconnect,
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct PeerIo {
    addr: SocketAddr,
    message_rx: mpsc::UnboundedReceiver<PeerMessage>,
    command_tx: mpsc::UnboundedSender<PeerIoCommand>,
    read_handle: JoinHandle<()>,
    write_handle: JoinHandle<()>,
    disconnected: bool,
    supports_extended: bool,
}

#[allow(dead_code)]
impl PeerIo {
    pub async fn connect(
        addr: SocketAddr,
        info_hash: InfoHashV1,
        peer_id: PeerId,
        connect_timeout: Duration,
    ) -> Result<Self, PeerWireError> {
        let stream = timeout(connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| {
                PeerWireError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "connect timeout",
                ))
            })?
            .map_err(PeerWireError::Io)?;

        let (mut read_half, mut write_half) = stream.into_split();

        let our_handshake = Handshake {
            reserved: ExtensionBits::default().with_extended(),
            info_hash,
            peer_id,
        };

        write_handshake(&mut write_half, &our_handshake).await?;
        let peer_handshake = read_handshake(&mut read_half, info_hash).await?;

        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let (command_tx, command_rx) = mpsc::unbounded_channel();

        let read_handle = tokio::spawn(read_loop(read_half, message_tx));
        let write_handle = tokio::spawn(write_loop(write_half, command_rx));

        Ok(Self {
            addr,
            message_rx,
            command_tx,
            read_handle,
            write_handle,
            disconnected: false,
            supports_extended: peer_handshake.reserved.supports_extended(),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn send(&self, msg: PeerMessage) -> Result<(), ()> {
        self.command_tx
            .send(PeerIoCommand::Send(msg))
            .map_err(|_| ())
    }

    pub fn supports_extended(&self) -> bool {
        self.supports_extended
    }

    pub fn drain(&mut self) -> Vec<PeerMessage> {
        let mut messages = Vec::new();
        loop {
            match self.message_rx.try_recv() {
                Ok(msg) => messages.push(msg),
                Err(TryRecvError::Disconnected) => {
                    self.disconnected = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        messages
    }

    pub fn disconnect(&mut self) {
        if self.disconnected {
            return;
        }
        self.disconnected = true;
        let _ = self.command_tx.send(PeerIoCommand::Disconnect);
        self.read_handle.abort();
        self.write_handle.abort();
    }

    pub fn is_disconnected(&self) -> bool {
        self.disconnected
    }
}

impl Drop for PeerIo {
    fn drop(&mut self) {
        self.disconnect();
    }
}

#[allow(dead_code)]
async fn read_loop(
    mut read_half: tokio::net::tcp::OwnedReadHalf,
    tx: mpsc::UnboundedSender<PeerMessage>,
) {
    while let Ok(msg) = read_message(&mut read_half, DEFAULT_MAX_PEER_FRAME_LEN).await {
        if tx.send(msg).is_err() {
            break;
        }
    }
}

#[allow(dead_code)]
async fn write_loop(
    mut write_half: tokio::net::tcp::OwnedWriteHalf,
    mut rx: mpsc::UnboundedReceiver<PeerIoCommand>,
) {
    use tokio::io::AsyncWriteExt;

    while let Some(cmd) = rx.recv().await {
        match cmd {
            PeerIoCommand::Send(msg) => {
                let Ok(bytes) = styx_proto::encode_message(&msg) else {
                    continue;
                };
                if write_half.write_all(&bytes).await.is_err() {
                    break;
                }
            }
            PeerIoCommand::Disconnect => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use styx_proto::{
        decode_handshake, encode_handshake, ExtensionBits, Handshake, InfoHashV1, PeerId,
        PeerMessage, PeerWireError, PEER_HANDSHAKE_LEN,
    };
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    use super::*;

    fn test_info_hash() -> InfoHashV1 {
        InfoHashV1::new([7u8; 20])
    }

    fn test_peer_id(byte: u8) -> PeerId {
        PeerId::new([byte; 20])
    }

    async fn mock_peer(
        listener: TcpListener,
        expected_info_hash: InfoHashV1,
        our_peer_id: PeerId,
        on_connected: oneshot::Sender<()>,
    ) {
        let (mut stream, _) = listener.accept().await.unwrap();

        let mut buf = [0u8; PEER_HANDSHAKE_LEN];
        tokio::io::AsyncReadExt::read_exact(&mut stream, &mut buf)
            .await
            .unwrap();
        let _peer_handshake = decode_handshake(&buf).unwrap();
        assert_eq!(_peer_handshake.info_hash, expected_info_hash);

        let handshake = Handshake {
            reserved: ExtensionBits::default(),
            info_hash: expected_info_hash,
            peer_id: our_peer_id,
        };
        let encoded = encode_handshake(&handshake);
        tokio::io::AsyncWriteExt::write_all(&mut stream, &encoded)
            .await
            .unwrap();

        let _ = on_connected.send(());
    }

    #[tokio::test]
    async fn t1_t1_peer_io_connects_and_exchanges_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let info_hash = test_info_hash();
        let peer_id = test_peer_id(1);

        let (tx_done, rx_done) = oneshot::channel();
        let mock_info_hash = info_hash;
        let mock_peer_id = test_peer_id(2);
        let mock = tokio::spawn(async move {
            mock_peer(listener, mock_info_hash, mock_peer_id, tx_done).await;
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });

        let mut io = PeerIo::connect(addr, info_hash, peer_id, Duration::from_secs(5))
            .await
            .expect("connect should succeed");

        let _ = tokio::time::timeout(Duration::from_secs(2), rx_done)
            .await
            .expect("handshake should complete");

        io.send(PeerMessage::Interested).unwrap();

        io.disconnect();
        mock.abort();
    }

    #[tokio::test]
    async fn t1_t2_connect_timeout_returns_error() {
        let info_hash = test_info_hash();
        let peer_id = test_peer_id(1);

        let result = PeerIo::connect(
            SocketAddr::from(([127, 0, 0, 1], 1)),
            info_hash,
            peer_id,
            Duration::from_millis(1),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn t1_t3_invalid_handshake_disconnects() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let info_hash = test_info_hash();
        let peer_id = test_peer_id(1);
        let wrong_info_hash = InfoHashV1::new([99u8; 20]);

        let (tx_done, rx_done) = oneshot::channel();
        let mock_info_hash = info_hash;
        let mock_wrong = wrong_info_hash;
        let mock_peer_id = test_peer_id(2);
        let mock = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();

            let mut buf = [0u8; PEER_HANDSHAKE_LEN];
            tokio::io::AsyncReadExt::read_exact(&mut stream, &mut buf)
                .await
                .unwrap();
            let _peer_hs = decode_handshake(&buf).unwrap();
            assert_eq!(_peer_hs.info_hash, mock_info_hash);

            let handshake = Handshake {
                reserved: ExtensionBits::default(),
                info_hash: mock_wrong,
                peer_id: mock_peer_id,
            };
            let encoded = encode_handshake(&handshake);
            tokio::io::AsyncWriteExt::write_all(&mut stream, &encoded)
                .await
                .unwrap();

            let _ = tx_done.send(());
        });

        let result = PeerIo::connect(addr, info_hash, peer_id, Duration::from_secs(5)).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(PeerWireError::InfoHashMismatch)));

        let _ = tokio::time::timeout(Duration::from_secs(2), rx_done).await;
        mock.await.unwrap();
    }

    #[tokio::test]
    async fn t1_t4_peer_disconnect_detected() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let info_hash = test_info_hash();
        let peer_id = test_peer_id(1);

        let (tx_connected, rx_connected) = oneshot::channel();
        let mock_info_hash = info_hash;
        let mock_peer_id = test_peer_id(2);
        let mock = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();

            let mut buf = [0u8; PEER_HANDSHAKE_LEN];
            tokio::io::AsyncReadExt::read_exact(&mut stream, &mut buf)
                .await
                .unwrap();
            let _peer_hs = decode_handshake(&buf).unwrap();

            let handshake = Handshake {
                reserved: ExtensionBits::default(),
                info_hash: mock_info_hash,
                peer_id: mock_peer_id,
            };
            let encoded = encode_handshake(&handshake);
            tokio::io::AsyncWriteExt::write_all(&mut stream, &encoded)
                .await
                .unwrap();

            let _ = tx_connected.send(());

            // Wait briefly then drop connection
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(stream);
        });

        let mut io = PeerIo::connect(addr, info_hash, peer_id, Duration::from_secs(5))
            .await
            .expect("connect should succeed");

        let _ = tokio::time::timeout(Duration::from_secs(2), rx_connected)
            .await
            .expect("handshake should complete");

        // Wait for peer to drop
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Drain should eventually yield nothing (read loop detected disconnect)
        let msgs = io.drain();
        assert!(
            msgs.is_empty(),
            "no messages should be received from disconnected peer: got {msgs:?}"
        );

        io.disconnect();
        mock.await.unwrap();
    }
}
