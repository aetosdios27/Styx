//! Typed BitTorrent v1 metainfo parsing.

use std::collections::BTreeMap;
use std::ops::Range;

use bytes::Bytes;
use sha1::{Digest, Sha1};

use crate::bencode::{decode_top_level_dict_entries, BencodeError, BencodeValue};

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
    /// Parsed `info` dictionary.
    pub info: TorrentInfo,
    /// SHA-1 hash of the exact bencoded `info` dictionary bytes.
    pub info_hash_v1: InfoHashV1,
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
    pub pieces: Bytes,
    /// Optional private torrent flag.
    pub private: bool,
    /// Single-file or multi-file layout.
    pub mode: FileMode,
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
    let info = parse_info(&info_entry.value)?;
    let raw_info = raw_slice(input, info_entry.value_span.clone());
    let info_hash_v1 = sha1_digest(&raw_info);

    Ok(TorrentMetainfo {
        announce,
        announce_list,
        info,
        info_hash_v1,
        raw_info,
    })
}

fn parse_info(value: &BencodeValue) -> Result<TorrentInfo, TorrentMetainfoError> {
    let dict = expect_dict(value, "info")?;
    let name = required_non_empty_bytes(dict, b"name", "info")?;
    let piece_length = required_positive_u64(dict, b"piece length", "info")?;
    let pieces = required_non_empty_bytes(dict, b"pieces", "info")?;
    if pieces.len() % SHA1_DIGEST_BYTES != 0 {
        return Err(TorrentMetainfoError::InvalidPiecesLength);
    }

    let private = optional_boolish_int(dict, b"private", "info")?.unwrap_or(false);
    let length = optional_non_negative_u64(dict, b"length", "info")?;
    let files = optional_files(dict)?;
    let mode = match (length, files) {
        (Some(length), None) => FileMode::Single { length },
        (None, Some(files)) => FileMode::Multi { files },
        _ => return Err(TorrentMetainfoError::InvalidFileMode),
    };

    Ok(TorrentInfo {
        name,
        piece_length,
        pieces,
        private,
        mode,
    })
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
