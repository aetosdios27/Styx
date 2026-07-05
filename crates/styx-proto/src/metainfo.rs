//! Typed BitTorrent v1 metainfo parsing.

use std::collections::BTreeMap;
use std::ops::Range;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use sha2::Sha256;

use crate::bencode::{decode_top_level_dict_entries, BencodeError, BencodeValue};
use crate::{InfoHashV2, SHA256_DIGEST_BYTES};

const SHA1_DIGEST_BYTES: usize = 20;

/// A BitTorrent v1 info hash.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InfoHashV1([u8; SHA1_DIGEST_BYTES]);

impl InfoHashV1 {
    /// Construct a v1 info hash from its raw 20-byte SHA-1 digest.
    #[must_use]
    pub const fn new(bytes: [u8; SHA1_DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    /// Return the raw 20-byte SHA-1 digest.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; SHA1_DIGEST_BYTES] {
        &self.0
    }
}

/// A parsed `.torrent` metainfo file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TorrentMetainfo {
    /// Primary tracker announce URL, when present.
    pub announce: Option<Bytes>,
    /// BEP 12 announce tiers.
    pub announce_list: Vec<Vec<Bytes>>,
    /// BEP 19 web seed URLs from `url-list`.
    pub url_list: Vec<Bytes>,
    /// Parsed `info` dictionary.
    pub info: TorrentInfo,
    /// SHA-1 hash of the exact bencoded `info` dictionary bytes.
    pub info_hash_v1: InfoHashV1,
    /// SHA-256 hash of the exact bencoded `info` dictionary bytes (BEP 52).
    /// Always computed for every torrent, regardless of v1/v2.
    pub info_hash_v2: Option<InfoHashV2>,
    /// BEP 52 piece layers: maps pieces_root → per-piece Merkle root hashes.
    pub piece_layers: Option<BTreeMap<InfoHashV2, Vec<u8>>>,
    /// Exact bencoded `info` dictionary bytes from the source file.
    pub raw_info: Bytes,
}

/// Parsed v1 torrent info dictionary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TorrentInfo {
    /// Display name from the `name` field.
    pub name: Bytes,
    /// Piece length in bytes.
    pub piece_length: u64,
    /// Concatenated 20-byte SHA-1 piece hashes.
    /// `None` for v2-only torrents (BEP 52 info dict omits this key).
    pub pieces: Option<Bytes>,
    /// Optional private torrent flag.
    pub private: bool,
    /// Single-file or multi-file layout.
    pub mode: FileMode,
    /// BEP 52 meta version. `Some(2)` for v2/hybrid, `None` for v1.
    pub meta_version: Option<u32>,
    /// BEP 52 file tree (v2/hybrid only).
    pub file_tree: Option<crate::file_tree::V2FileTree>,
}

/// File layout stored in the v1 info dictionary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileMode {
    /// Single-file torrent with total byte length.
    Single { length: u64 },
    /// Multi-file torrent with per-file paths and lengths.
    Multi { files: Vec<TorrentFile> },
}

/// A file entry inside a multi-file torrent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TorrentFile {
    /// File length in bytes.
    pub length: u64,
    /// Path components as raw bytes.
    pub path: Vec<Bytes>,
}

/// Errors returned while parsing `.torrent` metainfo.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TorrentMetainfoError {
    /// Bencode parsing failed.
    #[error(transparent)]
    Bencode(#[from] BencodeError),
    /// The top-level dictionary is missing `info`.
    #[error("missing top-level info dictionary")]
    MissingInfo,
    /// A required field is absent.
    #[error("missing field `{field}` in {context}")]
    MissingField {
        /// Field name.
        field: &'static str,
        /// Structure where the field was required.
        context: &'static str,
    },
    /// A field had the wrong bencode type.
    #[error("field `{field}` in {context} has the wrong type")]
    WrongType {
        /// Field name.
        field: &'static str,
        /// Structure where the field was read.
        context: &'static str,
    },
    /// A numeric field was outside the accepted range.
    #[error("field `{field}` in {context} is outside the valid range")]
    InvalidIntegerRange {
        /// Field name.
        field: &'static str,
        /// Structure where the field was read.
        context: &'static str,
    },
    /// The `pieces` field is not a whole number of SHA-1 hashes.
    #[error("info.pieces length must be a non-empty multiple of 20 bytes")]
    InvalidPiecesLength,
    /// Multi-file and single-file fields were mixed or missing.
    #[error("info dictionary must contain exactly one of `length` or `files`")]
    InvalidFileMode,
    /// A list field was empty where BEP 3 requires content.
    #[error("field `{field}` in {context} must not be empty")]
    EmptyList {
        /// Field name.
        field: &'static str,
        /// Structure where the field was read.
        context: &'static str,
    },
    /// A byte string field was empty where content is required.
    #[error("field `{field}` in {context} must not be an empty byte string")]
    EmptyBytes {
        /// Field name.
        field: &'static str,
        /// Structure where the field was read.
        context: &'static str,
    },
    /// A multi-file path component would be unsafe to map onto disk.
    #[error("file path component is unsafe")]
    UnsafePathComponent,
    /// The BEP 52 file tree could not be parsed.
    #[error("invalid v2 file tree")]
    InvalidFileTree,
    /// Hybrid torrent v1 and v2 file metadata are inconsistent.
    #[error("hybrid torrent v1/v2 file metadata mismatch: {0}")]
    HybridInconsistent(String),
}

/// Decode a v1 `.torrent` metainfo document.
///
/// # Errors
///
/// Returns [`TorrentMetainfoError`] when the input is invalid bencode or does
/// not satisfy the v1 metainfo shape required by BEP 3.
pub fn decode_torrent(input: &[u8]) -> Result<TorrentMetainfo, TorrentMetainfoError> {
    let entries = decode_top_level_dict_entries(input)?;
    let info_entry = entries
        .iter()
        .find(|entry| entry.key == b"info")
        .ok_or(TorrentMetainfoError::MissingInfo)?;

    let announce = optional_bytes(&entries, b"announce", "metainfo")?;
    let announce_list = optional_announce_list(&entries)?;
    let url_list = optional_url_list(&entries)?;
    let info = parse_info(&info_entry.value)?;
    let raw_info = raw_slice(input, info_entry.value_span.clone());
    let info_hash_v1 = sha1_digest(&raw_info);
    let info_hash_v2 = Some(sha256_digest(&raw_info));
    let piece_layers = parse_piece_layers(&entries);

    if info.pieces.is_some() && info.file_tree.is_some() {
        let v2_files = info
            .file_tree
            .as_ref()
            .unwrap()
            .flatten()
            .map_err(|e| TorrentMetainfoError::HybridInconsistent(e.to_string()))?;
        crate::hybrid::validate_hybrid_consistency(&info, &v2_files).map_err(|e| {
            TorrentMetainfoError::HybridInconsistent(e.to_string())
        })?;
    }

    Ok(TorrentMetainfo {
        announce,
        announce_list,
        url_list,
        info,
        info_hash_v1,
        info_hash_v2,
        piece_layers,
        raw_info,
    })
}

fn parse_piece_layers(
    entries: &[crate::bencode::SpannedDictEntry],
) -> Option<BTreeMap<InfoHashV2, Vec<u8>>> {
    let value = entries
        .iter()
        .find(|e| e.key == b"piece layers")?
        .value
        .clone();
    let dict = match &value {
        BencodeValue::Dict(d) => d,
        _ => return None,
    };
    Some(
        dict.iter()
            .filter_map(|(key, value)| {
                if key.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(key);
                    let hash = InfoHashV2::new(arr);
                    match value {
                        BencodeValue::Bytes(b) => Some((hash, b.clone().to_vec())),
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect(),
    )
}

fn optional_url_list(
    entries: &[crate::bencode::SpannedDictEntry],
) -> Result<Vec<Bytes>, TorrentMetainfoError> {
    let Some(value) = top_level_value(entries, b"url-list") else {
        return Ok(Vec::new());
    };
    match value {
        BencodeValue::Bytes(bytes) => {
            if bytes.is_empty() {
                return Err(TorrentMetainfoError::EmptyBytes {
                    field: "url-list",
                    context: "metainfo",
                });
            }
            Ok(vec![bytes.clone()])
        }
        BencodeValue::List(values) => {
            if values.is_empty() {
                return Err(TorrentMetainfoError::EmptyList {
                    field: "url-list",
                    context: "metainfo",
                });
            }
            values
                .iter()
                .map(|value| match value {
                    BencodeValue::Bytes(bytes) if !bytes.is_empty() => Ok(bytes.clone()),
                    BencodeValue::Bytes(_) => Err(TorrentMetainfoError::EmptyBytes {
                        field: "url-list",
                        context: "metainfo",
                    }),
                    _ => Err(TorrentMetainfoError::WrongType {
                        field: "url-list",
                        context: "metainfo",
                    }),
                })
                .collect()
        }
        _ => Err(TorrentMetainfoError::WrongType {
            field: "url-list",
            context: "metainfo",
        }),
    }
}

fn parse_info(value: &BencodeValue) -> Result<TorrentInfo, TorrentMetainfoError> {
    let dict = expect_dict(value, "info")?;
    let name = required_non_empty_bytes(dict, b"name", "info")?;
    let piece_length = required_positive_u64(dict, b"piece length", "info")?;
    let pieces = match dict.get(&b"pieces"[..]) {
        Some(BencodeValue::Bytes(pieces)) if !pieces.is_empty() => {
            if pieces.len() % SHA1_DIGEST_BYTES != 0 {
                return Err(TorrentMetainfoError::InvalidPiecesLength);
            }
            Some(pieces.clone())
        }
        Some(BencodeValue::Bytes(_)) => {
            return Err(TorrentMetainfoError::EmptyBytes {
                field: "pieces",
                context: "info",
            });
        }
        Some(_) => {
            return Err(TorrentMetainfoError::WrongType {
                field: "pieces",
                context: "info",
            });
        }
        None => None,
    };

    let private = optional_boolish_int(dict, b"private", "info")?.unwrap_or(false);
    let length = optional_non_negative_u64(dict, b"length", "info")?;
    let files = optional_files(dict)?;
    let meta_version = parse_meta_version(dict);
    let file_tree = if meta_version == Some(2) {
        parse_file_tree(dict)?
    } else {
        None
    };
    let mode = match (length, files, meta_version) {
        (Some(length), None, _) => FileMode::Single { length },
        (None, Some(files), _) => FileMode::Multi { files },
        (None, None, Some(2)) => FileMode::Single { length: 0 },
        _ => return Err(TorrentMetainfoError::InvalidFileMode),
    };

    Ok(TorrentInfo {
        name,
        piece_length,
        pieces,
        private,
        mode,
        meta_version,
        file_tree,
    })
}

fn parse_meta_version(dict: &BTreeMap<Vec<u8>, BencodeValue>) -> Option<u32> {
    dict.get(b"meta version".as_slice()).and_then(|v| match v {
        BencodeValue::Integer(i) => Some(*i as u32),
        _ => None,
    })
}

fn parse_file_tree(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
) -> Result<Option<crate::file_tree::V2FileTree>, TorrentMetainfoError> {
    match dict.get(b"file tree".as_slice()) {
        Some(value) => {
            let ft = crate::file_tree::V2FileTree::from_bencode(value)
                .map_err(|_| TorrentMetainfoError::InvalidFileTree)?;
            Ok(Some(ft))
        }
        None => Ok(None),
    }
}

fn optional_files(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
) -> Result<Option<Vec<TorrentFile>>, TorrentMetainfoError> {
    let Some(value) = dict.get(b"files".as_slice()) else {
        return Ok(None);
    };
    let files = expect_list(value, "files", "info")?;
    if files.is_empty() {
        return Err(TorrentMetainfoError::EmptyList {
            field: "files",
            context: "info",
        });
    }

    files
        .iter()
        .map(parse_file)
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn parse_file(value: &BencodeValue) -> Result<TorrentFile, TorrentMetainfoError> {
    let dict = expect_dict(value, "file")?;
    let length = required_non_negative_u64(dict, b"length", "file")?;
    let path_value = required_field(dict, b"path", "file")?;
    let components = expect_list(path_value, "path", "file")?;
    if components.is_empty() {
        return Err(TorrentMetainfoError::EmptyList {
            field: "path",
            context: "file",
        });
    }

    let path = components
        .iter()
        .map(|component| match component {
            BencodeValue::Bytes(bytes) if is_safe_path_component(bytes) => Ok(bytes.clone()),
            BencodeValue::Bytes(bytes) if bytes.is_empty() => {
                Err(TorrentMetainfoError::EmptyBytes {
                    field: "path",
                    context: "file",
                })
            }
            BencodeValue::Bytes(_) => Err(TorrentMetainfoError::UnsafePathComponent),
            _ => Err(TorrentMetainfoError::WrongType {
                field: "path",
                context: "file",
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(TorrentFile { length, path })
}

fn is_safe_path_component(component: &[u8]) -> bool {
    !component.is_empty()
        && component != b"."
        && component != b".."
        && !component
            .iter()
            .any(|byte| matches!(byte, b'\0' | b'/' | b'\\'))
}

fn optional_announce_list(
    entries: &[crate::bencode::SpannedDictEntry],
) -> Result<Vec<Vec<Bytes>>, TorrentMetainfoError> {
    let Some(value) = top_level_value(entries, b"announce-list") else {
        return Ok(Vec::new());
    };

    let tiers = expect_list(value, "announce-list", "metainfo")?;
    tiers
        .iter()
        .map(|tier| {
            let urls = expect_list(tier, "announce-list tier", "metainfo")?;
            if urls.is_empty() {
                return Err(TorrentMetainfoError::EmptyList {
                    field: "announce-list tier",
                    context: "metainfo",
                });
            }
            urls.iter()
                .map(|url| match url {
                    BencodeValue::Bytes(bytes) if !bytes.is_empty() => Ok(bytes.clone()),
                    BencodeValue::Bytes(_) => Err(TorrentMetainfoError::EmptyBytes {
                        field: "announce-list",
                        context: "metainfo",
                    }),
                    _ => Err(TorrentMetainfoError::WrongType {
                        field: "announce-list",
                        context: "metainfo",
                    }),
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .collect()
}

fn optional_bytes(
    entries: &[crate::bencode::SpannedDictEntry],
    field: &'static [u8],
    context: &'static str,
) -> Result<Option<Bytes>, TorrentMetainfoError> {
    top_level_value(entries, field)
        .map(|value| match value {
            BencodeValue::Bytes(bytes) if !bytes.is_empty() => Ok(bytes.clone()),
            BencodeValue::Bytes(_) => Err(TorrentMetainfoError::EmptyBytes {
                field: field_name(field),
                context,
            }),
            _ => Err(TorrentMetainfoError::WrongType {
                field: field_name(field),
                context,
            }),
        })
        .transpose()
}

fn top_level_value<'a>(
    entries: &'a [crate::bencode::SpannedDictEntry],
    key: &[u8],
) -> Option<&'a BencodeValue> {
    entries
        .iter()
        .find(|entry| entry.key.as_slice() == key)
        .map(|entry| &entry.value)
}

fn expect_dict<'a>(
    value: &'a BencodeValue,
    context: &'static str,
) -> Result<&'a BTreeMap<Vec<u8>, BencodeValue>, TorrentMetainfoError> {
    match value {
        BencodeValue::Dict(dict) => Ok(dict),
        _ => Err(TorrentMetainfoError::WrongType {
            field: context,
            context,
        }),
    }
}

fn expect_list<'a>(
    value: &'a BencodeValue,
    field: &'static str,
    context: &'static str,
) -> Result<&'a [BencodeValue], TorrentMetainfoError> {
    match value {
        BencodeValue::List(values) => Ok(values),
        _ => Err(TorrentMetainfoError::WrongType { field, context }),
    }
}

fn required_field<'a>(
    dict: &'a BTreeMap<Vec<u8>, BencodeValue>,
    field: &'static [u8],
    context: &'static str,
) -> Result<&'a BencodeValue, TorrentMetainfoError> {
    dict.get(field).ok_or(TorrentMetainfoError::MissingField {
        field: field_name(field),
        context,
    })
}

fn required_non_empty_bytes(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
    field: &'static [u8],
    context: &'static str,
) -> Result<Bytes, TorrentMetainfoError> {
    match required_field(dict, field, context)? {
        BencodeValue::Bytes(bytes) if !bytes.is_empty() => Ok(bytes.clone()),
        BencodeValue::Bytes(_) => Err(TorrentMetainfoError::EmptyBytes {
            field: field_name(field),
            context,
        }),
        _ => Err(TorrentMetainfoError::WrongType {
            field: field_name(field),
            context,
        }),
    }
}

fn required_positive_u64(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
    field: &'static [u8],
    context: &'static str,
) -> Result<u64, TorrentMetainfoError> {
    match required_non_negative_u64(dict, field, context)? {
        0 => Err(TorrentMetainfoError::InvalidIntegerRange {
            field: field_name(field),
            context,
        }),
        value => Ok(value),
    }
}

fn required_non_negative_u64(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
    field: &'static [u8],
    context: &'static str,
) -> Result<u64, TorrentMetainfoError> {
    let Some(value) = optional_non_negative_u64(dict, field, context)? else {
        return Err(TorrentMetainfoError::MissingField {
            field: field_name(field),
            context,
        });
    };
    Ok(value)
}

fn optional_non_negative_u64(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
    field: &'static [u8],
    context: &'static str,
) -> Result<Option<u64>, TorrentMetainfoError> {
    let Some(value) = dict.get(field) else {
        return Ok(None);
    };
    match value {
        BencodeValue::Integer(integer) => {
            (*integer)
                .try_into()
                .map(Some)
                .map_err(|_| TorrentMetainfoError::InvalidIntegerRange {
                    field: field_name(field),
                    context,
                })
        }
        _ => Err(TorrentMetainfoError::WrongType {
            field: field_name(field),
            context,
        }),
    }
}

fn optional_boolish_int(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
    field: &'static [u8],
    context: &'static str,
) -> Result<Option<bool>, TorrentMetainfoError> {
    let Some(value) = dict.get(field) else {
        return Ok(None);
    };
    match value {
        BencodeValue::Integer(0) => Ok(Some(false)),
        BencodeValue::Integer(1) => Ok(Some(true)),
        BencodeValue::Integer(_) => Err(TorrentMetainfoError::InvalidIntegerRange {
            field: field_name(field),
            context,
        }),
        _ => Err(TorrentMetainfoError::WrongType {
            field: field_name(field),
            context,
        }),
    }
}

fn raw_slice(input: &[u8], span: Range<usize>) -> Bytes {
    Bytes::copy_from_slice(&input[span])
}

fn sha1_digest(bytes: &[u8]) -> InfoHashV1 {
    let digest = Sha1::digest(bytes);
    let mut output = [0u8; SHA1_DIGEST_BYTES];
    output.copy_from_slice(&digest);
    InfoHashV1(output)
}

fn sha256_digest(bytes: &[u8]) -> InfoHashV2 {
    let digest = Sha256::digest(bytes);
    let mut output = [0u8; SHA256_DIGEST_BYTES];
    output.copy_from_slice(&digest);
    InfoHashV2::new(output)
}

fn field_name(field: &'static [u8]) -> &'static str {
    match field {
        b"announce" => "announce",
        b"announce-list" => "announce-list",
        b"files" => "files",
        b"info" => "info",
        b"length" => "length",
        b"name" => "name",
        b"path" => "path",
        b"piece length" => "piece length",
        b"pieces" => "pieces",
        b"private" => "private",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINGLE_FILE_TORRENT: &[u8] = include_bytes!("../tests/fixtures/single_file.torrent");
    const MULTI_FILE_TORRENT: &[u8] = include_bytes!("../tests/fixtures/multi_file.torrent");

    fn fixture(input: &'static [u8]) -> &'static [u8] {
        input.strip_suffix(b"\n").unwrap_or(input)
    }

    #[test]
    fn decode_torrent_parses_single_file_fixture() {
        let metainfo = decode_torrent(fixture(SINGLE_FILE_TORRENT)).unwrap();

        assert_eq!(
            metainfo.announce.as_deref(),
            Some(b"http://tracker.example/announce".as_slice())
        );
        assert_eq!(metainfo.info.name.as_ref(), b"hello.txt");
        assert_eq!(metainfo.info.piece_length, 16_384);
        assert_eq!(metainfo.info.mode, FileMode::Single { length: 12_345 });
        assert_eq!(metainfo.raw_info.as_ref(), b"d6:lengthi12345e4:name9:hello.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaae");
    }

    #[test]
    fn decode_torrent_hashes_exact_raw_info_bytes() {
        let metainfo = decode_torrent(fixture(SINGLE_FILE_TORRENT)).unwrap();

        assert_eq!(
            metainfo.info_hash_v1.as_bytes(),
            &[
                0x8f, 0xba, 0x42, 0x94, 0x27, 0x84, 0x64, 0x87, 0xc7, 0x30, 0x3d, 0xee, 0x26, 0x2f,
                0x88, 0x7f, 0x23, 0xe7, 0x49, 0xec,
            ]
        );
    }

    #[test]
    fn decode_torrent_computes_sha256_info_hash_v2() {
        let metainfo = decode_torrent(fixture(SINGLE_FILE_TORRENT)).unwrap();

        assert!(metainfo.info_hash_v2.is_some());
        assert_eq!(
            metainfo.info_hash_v2.unwrap().as_bytes(),
            &[
                0x48, 0xfe, 0x13, 0xad, 0x89, 0xde, 0x77, 0xe8, 0xb8, 0x89, 0x06, 0xba, 0xb3, 0x40,
                0xc6, 0x10, 0x5b, 0x04, 0xb6, 0x98, 0x51, 0x30, 0x09, 0x24, 0x89, 0xc7, 0x78, 0x62,
                0xa7, 0x6e, 0xc8, 0x1f,
            ]
        );
    }

    #[test]
    fn decode_torrent_parses_multi_file_fixture() {
        let metainfo = decode_torrent(fixture(MULTI_FILE_TORRENT)).unwrap();

        assert_eq!(metainfo.announce_list.len(), 1);
        assert_eq!(metainfo.info.name.as_ref(), b"album");
        assert_eq!(metainfo.info.piece_length, 32_768);
        assert!(metainfo.info.private);
        assert_eq!(
            metainfo.info.mode,
            FileMode::Multi {
                files: vec![
                    TorrentFile {
                        length: 3,
                        path: vec![Bytes::from_static(b"one.txt")]
                    },
                    TorrentFile {
                        length: 4,
                        path: vec![Bytes::from_static(b"dir"), Bytes::from_static(b"two.bin")]
                    },
                ]
            }
        );
    }

    #[test]
    fn parse_v2_file_tree_single_file() {
        let pieces_root = [b'a'; 32];
        let mut buf = b"d4:testd0:d6:lengthi1024e11:pieces root32:".to_vec();
        buf.extend_from_slice(&pieces_root);
        buf.extend_from_slice(b"eee");
        let parsed = crate::bencode::decode(&buf).unwrap();
        let ft = crate::file_tree::V2FileTree::from_bencode(&parsed).unwrap();
        let files = ft.flatten().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path_components, vec![b"test".to_vec()]);
        assert_eq!(files[0].entry.length, 1024);
    }

    #[test]
    fn decode_torrent_rejects_missing_info() {
        assert_eq!(
            decode_torrent(b"d8:announce3:urle"),
            Err(TorrentMetainfoError::MissingInfo)
        );
    }

    #[test]
    fn decode_torrent_rejects_invalid_piece_hash_length() {
        let err = decode_torrent(b"d4:infod6:lengthi1e4:name1:a12:piece lengthi16e6:pieces3:abcee")
            .unwrap_err();

        assert_eq!(err, TorrentMetainfoError::InvalidPiecesLength);
    }

    #[test]
    fn decode_torrent_rejects_mixed_file_modes() {
        let err = decode_torrent(
            b"d4:infod5:filesld6:lengthi1e4:pathl1:aeee6:lengthi1e4:name1:a12:piece lengthi16e6:pieces20:aaaaaaaaaaaaaaaaaaaaee",
        )
        .unwrap_err();

        assert_eq!(err, TorrentMetainfoError::InvalidFileMode);
    }

    #[test]
    fn decode_torrent_rejects_unsafe_path_components() {
        let err = decode_torrent(
            b"d4:infod5:filesld6:lengthi1e4:pathl2:..eee4:name1:a12:piece lengthi16e6:pieces20:aaaaaaaaaaaaaaaaaaaaee",
        )
        .unwrap_err();

        assert_eq!(err, TorrentMetainfoError::UnsafePathComponent);
    }
}
