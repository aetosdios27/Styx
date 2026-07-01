use std::net::SocketAddr;

use tokio::net::UdpSocket;

use crate::{DhtError, DhtMessage};

const MAX_DHT_PACKET_SIZE: usize = 2048;

#[derive(Debug)]
pub struct DhtSocket {
    socket: UdpSocket,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocketEvent {
    pub message: DhtMessage,
    pub source: SocketAddr,
}

impl DhtSocket {
    pub async fn bind(addr: SocketAddr) -> Result<Self, DhtError> {
        Ok(Self {
            socket: UdpSocket::bind(addr).await?,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, DhtError> {
        Ok(self.socket.local_addr()?)
    }

    pub async fn send_to(&self, message: &DhtMessage, target: SocketAddr) -> Result<(), DhtError> {
        self.socket.send_to(&message.encode()?, target).await?;
        Ok(())
    }

    pub async fn poll_once(&self) -> Result<SocketEvent, DhtError> {
        let mut buffer = [0_u8; MAX_DHT_PACKET_SIZE];
        let (len, source) = self.socket.recv_from(&mut buffer).await?;
        let message = DhtMessage::decode(&buffer[..len])?;
        Ok(SocketEvent { message, source })
    }
}
