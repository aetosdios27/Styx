use std::path::{Component, Path, PathBuf};

use bytes::Bytes;
use styx_proto::{FileMode, TorrentMetainfo};

use crate::{DiskError, PieceIndex};

const SHA1_DIGEST_BYTES: usize = 20;

/// A storage plan derived from validated torrent metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiskPlan {
    root: PathBuf,
    files: Vec<FileEntry>,
    piece_lengths: Vec<u32>,
    spans_by_piece: Vec<Vec<FileSpan>>,
    piece_hashes_v1: Vec<[u8; SHA1_DIGEST_BYTES]>,
}

impl DiskPlan {
    /// Build a disk plan from parsed torrent metadata and a destination root.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when piece lengths, piece hash counts, paths, or
    /// offset arithmetic are invalid.
    pub fn from_metainfo(
        meta: &TorrentMetainfo,
        root: impl AsRef<Path>,
    ) -> Result<Self, DiskError> {
        let root = root.as_ref().to_path_buf();
        let piece_length =
            u32::try_from(meta.info.piece_length).map_err(|_| DiskError::InvalidPieceLength)?;
        if piece_length == 0 {
            return Err(DiskError::InvalidPieceLength);
        }

        let files = match &meta.info.mode {
            FileMode::Single { length } => {
                let name = path_component_from_bytes(&meta.info.name)?;
                vec![FileEntry::new(root.join(name), 0, *length)]
            }
            FileMode::Multi { files } => {
                let base = path_component_from_bytes(&meta.info.name)?;
                let mut offset = 0_u64;
                files
                    .iter()
                    .map(|file| {
                        let mut path = root.join(&base);
                        for component in &file.path {
                            path.push(path_component_from_bytes(component)?);
                        }
                        ensure_relative_target(&root, &path)?;
                        let entry = FileEntry::new(path, offset, file.length);
                        offset = offset
                            .checked_add(file.length)
                            .ok_or(DiskError::IntegerOverflow)?;
                        Ok(entry)
                    })
                    .collect::<Result<Vec<_>, DiskError>>()?
            }
        };

        for file in &files {
            ensure_relative_target(&root, file.path())?;
        }

        let total_length = files.iter().try_fold(0_u64, |total, file| {
            total
                .checked_add(file.length)
                .ok_or(DiskError::IntegerOverflow)
        })?;
        let piece_count = piece_count(total_length, piece_length)?;
        let piece_hashes_v1 = piece_hashes(&meta.info.pieces)?;
        let actual_piece_hashes =
            u32::try_from(piece_hashes_v1.len()).map_err(|_| DiskError::IntegerOverflow)?;
        if actual_piece_hashes != piece_count {
            return Err(DiskError::InvalidPieceHashCount {
                expected: piece_count,
                actual: actual_piece_hashes,
            });
        }

        let mut piece_lengths = Vec::with_capacity(piece_count as usize);
        let mut spans_by_piece = Vec::with_capacity(piece_count as usize);
        for raw_piece in 0..piece_count {
            let piece_start = u64::from(raw_piece)
                .checked_mul(u64::from(piece_length))
                .ok_or(DiskError::IntegerOverflow)?;
            let remaining = total_length
                .checked_sub(piece_start)
                .ok_or(DiskError::IntegerOverflow)?;
            let current_piece_length = remaining.min(u64::from(piece_length));
            let current_piece_length =
                u32::try_from(current_piece_length).map_err(|_| DiskError::IntegerOverflow)?;
            piece_lengths.push(current_piece_length);
            spans_by_piece.push(spans_for_range(&files, piece_start, current_piece_length)?);
        }

        Ok(Self {
            root,
            files,
            piece_lengths,
            spans_by_piece,
            piece_hashes_v1,
        })
    }

    /// Number of pieces in the torrent.
    ///
    /// This value is validated during [`DiskPlan::from_metainfo`], so the
    /// conversion from `usize` to `u32` cannot fail at this point.
    #[must_use]
    pub fn piece_count(&self) -> u32 {
        u32::try_from(self.piece_lengths.len())
            .expect("piece count must fit in u32 after DiskPlan construction")
    }

    /// Length of a piece in bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError::InvalidPieceIndex`] when the piece is out of range.
    pub fn piece_length(&self, piece: PieceIndex) -> Result<u32, DiskError> {
        self.piece_lengths
            .get(piece.get() as usize)
            .copied()
            .ok_or_else(|| DiskError::InvalidPieceIndex {
                piece: piece.get(),
                piece_count: self.piece_count(),
            })
    }

    /// File spans covered by a piece.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError::InvalidPieceIndex`] when the piece is out of range.
    pub fn spans_for_piece(&self, piece: PieceIndex) -> Result<&[FileSpan], DiskError> {
        self.spans_by_piece
            .get(piece.get() as usize)
            .map(Vec::as_slice)
            .ok_or_else(|| DiskError::InvalidPieceIndex {
                piece: piece.get(),
                piece_count: self.piece_count(),
            })
    }

    /// Planned output files.
    #[must_use]
    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    /// Destination root for this plan.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn expected_v1_hash(
        &self,
        piece: PieceIndex,
    ) -> Result<&[u8; SHA1_DIGEST_BYTES], DiskError> {
        self.piece_hashes_v1
            .get(piece.get() as usize)
            .ok_or_else(|| DiskError::InvalidPieceIndex {
                piece: piece.get(),
                piece_count: self.piece_count(),
            })
    }
}

/// A file in the torrent's output layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry {
    path: PathBuf,
    torrent_offset: u64,
    length: u64,
}

impl FileEntry {
    fn new(path: PathBuf, torrent_offset: u64, length: u64) -> Self {
        Self {
            path,
            torrent_offset,
            length,
        }
    }

    /// Absolute or root-relative output path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Offset of this file in the torrent byte stream.
    #[must_use]
    pub const fn torrent_offset(&self) -> u64 {
        self.torrent_offset
    }

    /// File length in bytes.
    #[must_use]
    pub const fn length(&self) -> u64 {
        self.length
    }
}

/// Portion of a piece stored in one output file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileSpan {
    /// Index into [`DiskPlan::files`].
    pub file_index: usize,
    /// Offset inside the output file.
    pub file_offset: u64,
    /// Offset inside the piece.
    pub piece_offset: u32,
    /// Span length in bytes.
    pub length: u32,
}

fn piece_count(total_length: u64, piece_length: u32) -> Result<u32, DiskError> {
    if piece_length == 0 {
        return Err(DiskError::InvalidPieceLength);
    }
    if total_length == 0 {
        return Ok(0);
    }
    let piece_length = u64::from(piece_length);
    let count = total_length
        .checked_add(piece_length - 1)
        .ok_or(DiskError::IntegerOverflow)?
        / piece_length;
    u32::try_from(count).map_err(|_| DiskError::IntegerOverflow)
}

fn piece_hashes(bytes: &Bytes) -> Result<Vec<[u8; SHA1_DIGEST_BYTES]>, DiskError> {
    if !bytes.len().is_multiple_of(SHA1_DIGEST_BYTES) {
        return Err(DiskError::InvalidPieceHashCount {
            expected: 0,
            actual: 0,
        });
    }
    bytes
        .chunks_exact(SHA1_DIGEST_BYTES)
        .map(|chunk| {
            <[u8; SHA1_DIGEST_BYTES]>::try_from(chunk).map_err(|_| DiskError::IntegerOverflow)
        })
        .collect()
}

fn spans_for_range(
    files: &[FileEntry],
    piece_start: u64,
    piece_length: u32,
) -> Result<Vec<FileSpan>, DiskError> {
    let piece_end = piece_start
        .checked_add(u64::from(piece_length))
        .ok_or(DiskError::IntegerOverflow)?;
    let mut spans = Vec::new();
    for (file_index, file) in files.iter().enumerate() {
        let file_start = file.torrent_offset;
        let file_end = file_start
            .checked_add(file.length)
            .ok_or(DiskError::IntegerOverflow)?;
        let overlap_start = piece_start.max(file_start);
        let overlap_end = piece_end.min(file_end);
        if overlap_start >= overlap_end {
            continue;
        }
        let file_offset = overlap_start
            .checked_sub(file_start)
            .ok_or(DiskError::IntegerOverflow)?;
        let piece_offset = overlap_start
            .checked_sub(piece_start)
            .ok_or(DiskError::IntegerOverflow)?;
        let length = overlap_end
            .checked_sub(overlap_start)
            .ok_or(DiskError::IntegerOverflow)?;
        spans.push(FileSpan {
            file_index,
            file_offset,
            piece_offset: u32::try_from(piece_offset).map_err(|_| DiskError::IntegerOverflow)?,
            length: u32::try_from(length).map_err(|_| DiskError::IntegerOverflow)?,
        });
    }
    Ok(spans)
}

fn path_component_from_bytes(bytes: &Bytes) -> Result<String, DiskError> {
    if !is_safe_component(bytes) {
        return Err(DiskError::UnsafePath);
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| DiskError::UnsafePath)
}

fn is_safe_component(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes != b"."
        && bytes != b".."
        && !bytes
            .iter()
            .any(|byte| matches!(byte, b'\0' | b'/' | b'\\'))
}

fn ensure_relative_target(root: &Path, target: &Path) -> Result<(), DiskError> {
    if target
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(DiskError::UnsafePath);
    }
    if root.is_relative()
        && target
            .components()
            .any(|component| matches!(component, Component::RootDir))
    {
        return Err(DiskError::UnsafePath);
    }
    if root.is_absolute() && !target.starts_with(root) {
        return Err(DiskError::UnsafePath);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bytes::Bytes;
    use styx_proto::{FileMode, InfoHashV1, TorrentFile, TorrentInfo, TorrentMetainfo};

    use super::*;
    use crate::PieceIndex;

    #[test]
    fn single_file_layout_maps_final_piece_span() {
        let meta = metainfo(TorrentInfo {
            name: Bytes::from_static(b"image.iso"),
            piece_length: 16 * 1024,
            pieces: Bytes::from(vec![0_u8; 20 * 3]),
            private: false,
            mode: FileMode::Single { length: 40 * 1024 },
        });

        let plan = DiskPlan::from_metainfo(&meta, "/downloads").unwrap();

        assert_eq!(plan.piece_count(), 3);
        assert_eq!(plan.piece_length(PieceIndex::new(2)).unwrap(), 8 * 1024);
        assert_eq!(
            plan.spans_for_piece(PieceIndex::new(2)).unwrap(),
            &[FileSpan {
                file_index: 0,
                file_offset: 32 * 1024,
                piece_offset: 0,
                length: 8 * 1024,
            }]
        );
        assert_eq!(plan.files()[0].path(), Path::new("/downloads/image.iso"));
    }

    #[test]
    fn multi_file_layout_maps_piece_across_file_boundary() {
        let meta = metainfo(TorrentInfo {
            name: Bytes::from_static(b"album"),
            piece_length: 16 * 1024,
            pieces: Bytes::from(vec![0_u8; 20 * 2]),
            private: false,
            mode: FileMode::Multi {
                files: vec![
                    TorrentFile {
                        length: 10 * 1024,
                        path: vec![Bytes::from_static(b"a.bin")],
                    },
                    TorrentFile {
                        length: 10 * 1024,
                        path: vec![Bytes::from_static(b"b.bin")],
                    },
                ],
            },
        });

        let plan = DiskPlan::from_metainfo(&meta, "/downloads").unwrap();

        assert_eq!(
            plan.spans_for_piece(PieceIndex::new(0)).unwrap(),
            &[
                FileSpan {
                    file_index: 0,
                    file_offset: 0,
                    piece_offset: 0,
                    length: 10 * 1024,
                },
                FileSpan {
                    file_index: 1,
                    file_offset: 0,
                    piece_offset: 10 * 1024,
                    length: 6 * 1024,
                },
            ]
        );
        assert_eq!(
            plan.spans_for_piece(PieceIndex::new(1)).unwrap(),
            &[FileSpan {
                file_index: 1,
                file_offset: 6 * 1024,
                piece_offset: 0,
                length: 4 * 1024,
            }]
        );
    }

    #[test]
    fn layout_rejects_unsafe_torrent_name() {
        let meta = metainfo(TorrentInfo {
            name: Bytes::from_static(b".."),
            piece_length: 16 * 1024,
            pieces: Bytes::from(vec![0_u8; 20]),
            private: false,
            mode: FileMode::Single { length: 1 },
        });

        let err = DiskPlan::from_metainfo(&meta, "/downloads").unwrap_err();

        assert_eq!(err, DiskError::UnsafePath);
    }

    #[test]
    fn layout_rejects_piece_hash_count_mismatch() {
        let meta = metainfo(TorrentInfo {
            name: Bytes::from_static(b"image.iso"),
            piece_length: 16 * 1024,
            pieces: Bytes::from(vec![0_u8; 20]),
            private: false,
            mode: FileMode::Single { length: 40 * 1024 },
        });

        let err = DiskPlan::from_metainfo(&meta, "/downloads").unwrap_err();

        assert_eq!(
            err,
            DiskError::InvalidPieceHashCount {
                expected: 3,
                actual: 1
            }
        );
    }

    #[test]
    fn layout_rejects_malformed_piece_hash_bytes() {
        let meta = metainfo(TorrentInfo {
            name: Bytes::from_static(b"image.iso"),
            piece_length: 16 * 1024,
            pieces: Bytes::from(vec![0_u8; 21]),
            private: false,
            mode: FileMode::Single { length: 1 },
        });

        let err = DiskPlan::from_metainfo(&meta, "/downloads").unwrap_err();

        assert_eq!(
            err,
            DiskError::InvalidPieceHashCount {
                expected: 0,
                actual: 0
            }
        );
    }

    #[test]
    fn layout_rejects_zero_piece_length() {
        let meta = metainfo(TorrentInfo {
            name: Bytes::from_static(b"image.iso"),
            piece_length: 0,
            pieces: Bytes::from(vec![0_u8; 20]),
            private: false,
            mode: FileMode::Single { length: 1 },
        });

        let err = DiskPlan::from_metainfo(&meta, "/downloads").unwrap_err();

        assert_eq!(err, DiskError::InvalidPieceLength);
    }

    #[test]
    fn ensure_relative_target_rejects_parent_dir_with_absolute_root() {
        let root = std::path::Path::new("/downloads");
        let target = std::path::Path::new("/downloads/../etc/passwd");
        let err = super::ensure_relative_target(root, target);
        assert_eq!(err, Err(DiskError::UnsafePath));
    }

    #[test]
    fn ensure_relative_target_rejects_parent_dir_with_relative_root() {
        let root = std::path::Path::new("downloads");
        let target = std::path::Path::new("downloads/../etc/passwd");
        let err = super::ensure_relative_target(root, target);
        assert_eq!(err, Err(DiskError::UnsafePath));
    }

    #[test]
    fn ensure_relative_target_accepts_legitimate_absolute_path() {
        let root = std::path::Path::new("/downloads");
        let target = std::path::Path::new("/downloads/subdir/file.bin");
        assert!(super::ensure_relative_target(root, target).is_ok());
    }

    fn metainfo(info: TorrentInfo) -> TorrentMetainfo {
        TorrentMetainfo {
            announce: None,
            announce_list: Vec::new(),
            url_list: Vec::new(),
            info,
            info_hash_v1: InfoHashV1::new([0; 20]),
            raw_info: Bytes::new(),
        }
    }
}
