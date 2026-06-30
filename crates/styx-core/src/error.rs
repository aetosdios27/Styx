use crate::{BlockRequest, PeerKey};

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum CoreError {
    #[error("invalid peer manager config field `{field}`")]
    InvalidConfig { field: &'static str },
    #[error("peer {peer:?} already exists")]
    PeerAlreadyExists { peer: PeerKey },
    #[error("unknown peer {peer:?}")]
    UnknownPeer { peer: PeerKey },
    #[error("duplicate request {request:?}")]
    DuplicateRequest { request: BlockRequest },
    #[error("request pipeline full for peer {peer:?}")]
    PipelineFull { peer: PeerKey },
    #[error("invalid peer message: {reason}")]
    InvalidPeerMessage { reason: &'static str },
    #[error("invalid bitfield length: expected {expected_pieces} pieces, got {actual_bits} bits")]
    InvalidBitfieldLength {
        expected_pieces: usize,
        actual_bits: usize,
    },
    #[error("request is not in flight: {request:?}")]
    RequestNotInFlight { request: BlockRequest },
}
