//! uTP transport primitives for Styx.
//!
//! This crate owns BEP 29 packet parsing, connection state, reliability,
//! congestion control, and UDP socket demultiplexing. It intentionally does
//! not know about torrents, pieces, trackers, DHT, or peer scheduling policy.

mod connection;
mod error;
mod ledbat;
mod packet;
mod reorder;
mod resource;
mod retransmit;
mod rtt;
mod sack;
mod sim;
mod sizing;
mod socket;
mod types;

pub use connection::{ConnectionRole, ConnectionState, UtpConnection, UtpEvent};
pub use error::UtpError;
pub use ledbat::LedbatController;
pub use packet::{Extension, UtpPacket};
pub use reorder::{ReorderBuffer, ReorderOutcome};
pub use retransmit::{RetransmitQueue, SentPacket};
pub use rtt::RttEstimator;
pub use sack::SelectiveAck;
pub use sizing::PacketSizer;
pub use socket::{SocketEvent, UtpSocket};
pub use types::{
    ConnectionId, PacketType, SeqNr, TimestampMicros, WindowBytes, DEFAULT_MTU, HEADER_LEN,
    INITIAL_TIMEOUT, MAX_EXTENSION_BYTES, MAX_PACKET_SIZE, MIN_TIMEOUT, TARGET_DELAY, UTP_VERSION,
};
