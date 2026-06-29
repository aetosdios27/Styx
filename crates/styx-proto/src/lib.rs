//! BitTorrent protocol primitives and wire-format parsers.
//!
//! `styx-proto` keeps byte-level protocol correctness separate from transport,
//! disk, and peer-management behavior. Phase 2 provides strict BEP 3 bencode
//! parsing plus v1 torrent metainfo parsing.

pub mod bencode;
pub mod metainfo;

pub use bencode::{decode, decode_with_span, encode, BencodeError, BencodeValue, DecodedBencode};
pub use metainfo::{
    decode_torrent, FileMode, InfoHashV1, TorrentFile, TorrentInfo, TorrentMetainfo,
    TorrentMetainfoError,
};
