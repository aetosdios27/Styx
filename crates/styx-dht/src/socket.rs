use std::net::SocketAddr;

use tokio::net::UdpSocket;

use crate::{DhtError, DhtMessage, DhtRuntime, InfoHash, NodeAddr, RuntimeAction};

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

#[derive(Debug)]
pub struct DhtSocketRuntime {
    socket: DhtSocket,
    runtime: DhtRuntime,
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

impl DhtSocketRuntime {
    #[must_use]
    pub const fn new(socket: DhtSocket, runtime: DhtRuntime) -> Self {
        Self { socket, runtime }
    }

    pub async fn step_once(&mut self) -> Result<(), DhtError> {
        self.step_once_action().await.map(|_| ())
    }

    pub async fn step_once_action(&mut self) -> Result<RuntimeAction, DhtError> {
        let event = self.socket.poll_once().await?;
        let action = self.runtime.handle_message(
            event.message,
            NodeAddr::new(event.source),
            std::time::Instant::now(),
        )?;
        if let Some(response) = &action.response {
            self.socket.send_to(response, event.source).await?;
        }
        for (target, message) in &action.outbound {
            self.socket.send_to(message, target.socket_addr()).await?;
        }
        Ok(action)
    }

    pub async fn bootstrap_once(&mut self) -> Result<(), DhtError> {
        let outbound = self.runtime.start_bootstrap(std::time::Instant::now())?;
        for (target, message) in outbound {
            self.socket.send_to(&message, target.socket_addr()).await?;
        }
        self.step_once().await
    }

    pub async fn get_peers_once(&mut self, info_hash: InfoHash) -> Result<RuntimeAction, DhtError> {
        let outbound = self
            .runtime
            .start_get_peers(info_hash, std::time::Instant::now())?;
        for (target, message) in outbound {
            self.socket.send_to(&message, target.socket_addr()).await?;
        }
        self.step_once_action().await
    }

    pub async fn announce_peer_once(
        &mut self,
        info_hash: InfoHash,
        port: u16,
        implied_port: bool,
    ) -> Result<(), DhtError> {
        let outbound = self.runtime.start_announce_peer(
            info_hash,
            port,
            implied_port,
            std::time::Instant::now(),
        )?;
        for (target, message) in outbound {
            self.socket.send_to(&message, target.socket_addr()).await?;
        }
        Ok(())
    }

    #[must_use]
    pub const fn runtime(&self) -> &DhtRuntime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut DhtRuntime {
        &mut self.runtime
    }

    #[must_use]
    pub const fn socket(&self) -> &DhtSocket {
        &self.socket
    }
}
