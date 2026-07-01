use std::{
    collections::BTreeMap,
    net::SocketAddr,
    time::{Duration, Instant},
};

use tokio::net::UdpSocket;

use crate::{ConnectionId, UtpConnection, UtpError, UtpEvent, UtpPacket};

#[derive(Debug)]
pub enum SocketEvent {
    Packet {
        remote: SocketAddr,
        packet: UtpPacket,
    },
    ConnectionEvent {
        remote: SocketAddr,
        connection_id: ConnectionId,
        event: UtpEvent,
    },
}

#[derive(Debug)]
struct ConnectionEntry {
    connection: UtpConnection,
    last_seen: Instant,
}

#[derive(Debug)]
pub struct UtpSocket {
    socket: UdpSocket,
    connections: BTreeMap<(SocketAddr, ConnectionId), ConnectionEntry>,
    max_connections: usize,
    max_connections_per_remote: usize,
    stale_after: Duration,
}

impl UtpSocket {
    pub async fn bind(addr: SocketAddr) -> Result<Self, UtpError> {
        let socket = UdpSocket::bind(addr).await?;
        Ok(Self {
            socket,
            connections: BTreeMap::new(),
            max_connections: 1024,
            max_connections_per_remote: 16,
            stale_after: Duration::from_secs(120),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, UtpError> {
        Ok(self.socket.local_addr()?)
    }

    pub async fn send_packet(
        &self,
        remote: SocketAddr,
        packet: &UtpPacket,
    ) -> Result<usize, UtpError> {
        Ok(self.socket.send_to(&packet.encode(), remote).await?)
    }

    pub async fn poll_once(&mut self, now: Instant) -> Result<SocketEvent, UtpError> {
        self.evict_stale(now);
        let mut buf = vec![0; crate::MAX_PACKET_SIZE];
        let (len, remote) = self.socket.recv_from(&mut buf).await?;
        let packet = UtpPacket::decode(&buf[..len])?;
        let key = (remote, packet.connection_id());

        if let Some(entry) = self.connections.get_mut(&key) {
            entry.last_seen = now;
            let events = entry.connection.handle_packet(packet, now)?;
            let Some(event) = events.into_iter().next() else {
                return Err(UtpError::InvalidStateTransition);
            };
            return Ok(SocketEvent::ConnectionEvent {
                remote,
                connection_id: key.1,
                event,
            });
        }

        Ok(SocketEvent::Packet { remote, packet })
    }

    pub fn insert_connection(
        &mut self,
        remote: SocketAddr,
        connection_id: ConnectionId,
        connection: UtpConnection,
        now: Instant,
    ) -> Result<(), UtpError> {
        if self.connections.len() >= self.max_connections {
            return Err(UtpError::ResourceLimitExceeded {
                resource: "socket_connections",
            });
        }
        let remote_count = self
            .connections
            .keys()
            .filter(|(addr, _)| *addr == remote)
            .count();
        if remote_count >= self.max_connections_per_remote {
            return Err(UtpError::ResourceLimitExceeded {
                resource: "socket_connections_per_remote",
            });
        }
        self.connections.insert(
            (remote, connection_id),
            ConnectionEntry {
                connection,
                last_seen: now,
            },
        );
        Ok(())
    }

    pub fn evict_stale(&mut self, now: Instant) {
        self.connections
            .retain(|_, entry| now.duration_since(entry.last_seen) <= self.stale_after);
    }

    #[must_use]
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, time::Instant};

    use crate::{ConnectionId, SeqNr, UtpConnection};

    use super::*;

    #[tokio::test]
    #[ignore = "requires UDP socket permissions in the test environment"]
    async fn local_udp_sockets_exchange_syn_packet() {
        let a: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let left = UtpSocket::bind(a).await.unwrap();
        let mut right = UtpSocket::bind(b).await.unwrap();
        let remote = right.local_addr().unwrap();
        let (_conn, syn) =
            UtpConnection::client_syn(Instant::now(), ConnectionId::new(1), SeqNr::new(1)).unwrap();

        left.send_packet(remote, &syn).await.unwrap();
        let event = right.poll_once(Instant::now()).await.unwrap();

        assert!(matches!(event, SocketEvent::Packet { .. }));
    }

    #[tokio::test]
    #[ignore = "requires UDP socket permissions in the test environment"]
    async fn demux_keeps_connections_by_remote_and_connection_id() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let mut socket = UtpSocket::bind(addr).await.unwrap();
        let remote: SocketAddr = "127.0.0.1:9000".parse().unwrap();
        let (conn_a, _) =
            UtpConnection::client_syn(Instant::now(), ConnectionId::new(1), SeqNr::new(1)).unwrap();
        let (conn_b, _) =
            UtpConnection::client_syn(Instant::now(), ConnectionId::new(2), SeqNr::new(1)).unwrap();

        socket
            .insert_connection(remote, ConnectionId::new(1), conn_a, Instant::now())
            .unwrap();
        socket
            .insert_connection(remote, ConnectionId::new(2), conn_b, Instant::now())
            .unwrap();

        assert_eq!(socket.connection_count(), 2);
    }

    #[test]
    #[ignore = "requires UDP socket permissions in the test environment"]
    fn stale_connections_are_evicted() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
            let mut socket = UtpSocket::bind(addr).await.unwrap();
            let remote: SocketAddr = "127.0.0.1:9000".parse().unwrap();
            let now = Instant::now();
            let (conn, _) =
                UtpConnection::client_syn(now, ConnectionId::new(1), SeqNr::new(1)).unwrap();
            socket
                .insert_connection(remote, ConnectionId::new(1), conn, now)
                .unwrap();

            socket.evict_stale(now + Duration::from_secs(121));

            assert_eq!(socket.connection_count(), 0);
        });
    }
}
