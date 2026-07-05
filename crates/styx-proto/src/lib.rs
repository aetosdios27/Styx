//! BitTorrent protocol primitives and wire-format parsers.
//!
//! `styx-proto` keeps byte-level protocol correctness separate from transport,
//! disk, and peer-management behavior. It provides strict BEP 3 bencode and
//! v1 metainfo parsing plus peer-wire handshake and message framing.

pub mod bencode;
pub mod file_tree;
pub mod hash_msg;
pub mod hybrid;
pub mod info_hash_v2;
pub mod magnet_v2;
pub mod metainfo;
pub mod peer;

pub use bencode::{decode, decode_with_span, encode, BencodeError, BencodeValue, DecodedBencode};
pub use file_tree::{FileTreeError, V2FileEntry, V2FileTree, V2FileTreeNode, V2FlatFile};
pub use hash_msg::{
    HashMsgError, HashReject, HashRequest, HashesMessage, HASHES_ID, HASH_REJECT_ID,
    HASH_REQUEST_ID,
};
pub use hybrid::{is_hybrid, validate_hybrid_consistency, HybridError};
pub use info_hash_v2::{InfoHashV2, SHA256_DIGEST_BYTES};
pub use magnet_v2::{parse_v2_magnet, MagnetV2Error, V2MagnetInfo};
pub use metainfo::{
    decode_torrent, is_safe_path_component, FileMode, InfoHashV1, TorrentFile, TorrentInfo,
    TorrentMetainfo, TorrentMetainfoError,
};
pub use peer::{
    decode_handshake, decode_message_frame, decode_message_frame_with_limit, encode_handshake,
    encode_message, read_handshake, read_message, validate_handshake, write_handshake,
    write_message, ExtensionBits, Handshake, PeerId, PeerMessage, PeerWireError,
    DEFAULT_MAX_PEER_FRAME_LEN, PEER_HANDSHAKE_LEN,
};
