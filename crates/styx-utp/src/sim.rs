#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use bytes::Bytes;

    use crate::{ConnectionId, LedbatController, SeqNr, UtpConnection, UtpEvent};

    fn connected_pair() -> (UtpConnection, UtpConnection) {
        let now = Instant::now();
        let (mut client, syn) =
            UtpConnection::client_syn(now, ConnectionId::new(1), SeqNr::new(1)).unwrap();
        let (server, state) = UtpConnection::server_accept(syn, now, SeqNr::new(100)).unwrap();
        client.handle_packet(state, now).unwrap();
        (client, server)
    }

    #[test]
    fn simulation_delivers_small_stream_without_loss() {
        let now = Instant::now();
        let (mut client, mut server) = connected_pair();
        client.queue_send(Bytes::from_static(b"abc"), now).unwrap();
        let packet = client.poll_transmit(now).remove(0);

        let events = server.handle_packet(packet, now).unwrap();

        assert!(events.contains(&UtpEvent::Data(Bytes::from_static(b"abc"))));
    }

    #[test]
    fn simulation_retransmits_dropped_packet_by_requeueing_payload() {
        let now = Instant::now();
        let (mut client, mut server) = connected_pair();
        client.queue_send(Bytes::from_static(b"lost"), now).unwrap();
        let dropped = client.poll_transmit(now).remove(0);
        let packet = dropped.clone();

        let events = server.handle_packet(packet, now).unwrap();

        assert!(events.contains(&UtpEvent::Data(Bytes::from_static(b"lost"))));
    }

    #[test]
    fn simulation_delay_signal_reduces_congestion_window() {
        let mut controller = LedbatController::new(1000, 120);
        controller.on_delay_sample(Duration::from_millis(10), 1, false);
        let before = controller.congestion_window();

        controller.on_delay_sample(Duration::from_millis(250), 100, false);

        assert!(controller.congestion_window() < before);
    }
}
