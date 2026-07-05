//! BitTorrent protocol primitives and wire-format parsers.
//!
//! `styx-proto` keeps byte-level protocol correctness separate from transport,
//! disk, and peer-management behavior. It provides strict BEP 3 bencode and
//! v1 metainfo parsing plus peer-wire handshake and message framing.

pub mod bencode;
pub mod info_hash_v2;
pub mod metainfo;
pub mod peer;

pub use bencode::{decode, decode_with_span, encode, BencodeError, BencodeValue, DecodedBencode};
pub use info_hash_v2::{InfoHashV2, SHA256_DIGEST_BYTES};
pub use metainfo::{
    decode_torrent, FileMode, InfoHashV1, TorrentFile, TorrentInfo, TorrentMetainfo,
    TorrentMetainfoError,
};
pub use peer::{
    decode_handshake, decode_message_frame, decode_message_frame_with_limit, encode_handshake,
    encode_message, read_handshake, read_message, validate_handshake, write_handshake,
    write_message, ExtensionBits, Handshake, PeerId, PeerMessage, PeerWireError,
    DEFAULT_MAX_PEER_FRAME_LEN, PEER_HANDSHAKE_LEN,
};
