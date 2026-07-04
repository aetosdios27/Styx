use std::ffi::OsString;
use std::path::{Path, PathBuf};

use bytes::{Bytes, BytesMut};
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};

use crate::{DiskError, DiskPlan, FileSpan, PieceIndex};

/// Async disk IO for a planned torrent layout.
#[derive(Clone, Debug)]
pub struct DiskStore {
    plan: DiskPlan,
}

impl DiskStore {
    /// Construct a disk store from a validated plan.
    #[must_use]
    pub const fn new(plan: DiskPlan) -> Self {
        Self { plan }
    }

    /// Return the underlying disk plan.
    #[must_use]
    pub const fn plan(&self) -> &DiskPlan {
        &self.plan
    }

    /// Commit verified piece bytes to their mapped files.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when the piece is invalid, the payload length does
    /// not match the planned piece length, or disk IO fails.
    pub async fn commit_piece(&self, piece: PieceIndex, bytes: Bytes) -> Result<(), DiskError> {
        let piece_length = self.plan.piece_length(piece)?;
        let payload_length = u32::try_from(bytes.len()).map_err(|_| DiskError::IntegerOverflow)?;
        if payload_length != piece_length {
            return Err(DiskError::InvalidBlockLength {
                length: payload_length,
            });
        }

        let spans = self.plan.spans_for_piece(piece)?.to_vec();
        validate_span_slices(&spans, bytes.len())?;
        if spans.len() == 1 {
            self.commit_single_file_piece(piece, spans[0], &bytes).await
        } else {
            self.commit_multi_file_piece(&spans, &bytes).await
        }
    }

    /// Read a complete piece from disk.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when the piece is invalid or any mapped file read
    /// fails.
    pub async fn read_piece(&self, piece: PieceIndex) -> Result<Bytes, DiskError> {
        let piece_length = self.plan.piece_length(piece)?;
        let spans = self.plan.spans_for_piece(piece)?;
        let mut output = BytesMut::zeroed(piece_length as usize);
        for span in spans {
            let entry = &self.plan.files()[span.file_index];
            let mut file = OpenOptions::new().read(true).open(entry.path()).await?;
            file.seek(SeekFrom::Start(span.file_offset)).await?;
            let start = span.piece_offset as usize;
            let end = start
                .checked_add(span.length as usize)
                .ok_or(DiskError::IntegerOverflow)?;
            file.read_exact(&mut output[start..end]).await?;
        }
        Ok(output.freeze())
    }

    async fn commit_single_file_piece(
        &self,
        piece: PieceIndex,
        span: FileSpan,
        bytes: &[u8],
    ) -> Result<(), DiskError> {
        let entry = &self.plan.files()[span.file_index];
        let path = entry.path();
        ensure_parent(path).await?;

        let file_len = usize::try_from(entry.length()).map_err(|_| DiskError::IntegerOverflow)?;
        let mut staged = match fs::read(path).await {
            Ok(existing) => {
                let mut existing = existing;
                existing.resize(file_len, 0);
                existing
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => vec![0; file_len],
            Err(err) => return Err(DiskError::Io(err)),
        };
        let target_range = span_file_range(span)?;
        let piece_range = span_piece_range(span)?;
        staged[target_range].copy_from_slice(&bytes[piece_range]);

        let temp_path = temp_sibling_path(path, piece);
        let mut temp = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp_path)
            .await?;
        temp.write_all(&staged).await?;
        temp.flush().await?;
        drop(temp);
        fs::rename(&temp_path, path).await?;
        Ok(())
    }

    async fn commit_multi_file_piece(
        &self,
        spans: &[FileSpan],
        bytes: &[u8],
    ) -> Result<(), DiskError> {
        let mut staged = Vec::with_capacity(spans.len());
        for span in spans {
            let piece_range = span_piece_range(*span)?;
            staged.push((*span, Bytes::copy_from_slice(&bytes[piece_range])));
        }

        for (span, payload) in staged {
            let entry = &self.plan.files()[span.file_index];
            ensure_parent(entry.path()).await?;
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .read(true)
                .open(entry.path())
                .await?;
            file.seek(SeekFrom::Start(span.file_offset)).await?;
            file.write_all(&payload).await?;
            file.flush().await?;
        }

        Ok(())
    }
}

fn validate_span_slices(spans: &[FileSpan], bytes_len: usize) -> Result<(), DiskError> {
    for span in spans {
        let end = span
            .piece_offset
            .checked_add(span.length)
            .ok_or(DiskError::IntegerOverflow)?;
        if end as usize > bytes_len {
            return Err(DiskError::BlockOutOfBounds);
        }
    }
    Ok(())
}

fn span_piece_range(span: FileSpan) -> Result<std::ops::Range<usize>, DiskError> {
    let start = span.piece_offset as usize;
    let end = start
        .checked_add(span.length as usize)
        .ok_or(DiskError::IntegerOverflow)?;
    Ok(start..end)
}

fn span_file_range(span: FileSpan) -> Result<std::ops::Range<usize>, DiskError> {
    let start = usize::try_from(span.file_offset).map_err(|_| DiskError::IntegerOverflow)?;
    let end = start
        .checked_add(span.length as usize)
        .ok_or(DiskError::IntegerOverflow)?;
    Ok(start..end)
}

async fn ensure_parent(path: &Path) -> Result<(), DiskError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    Ok(())
}

fn temp_sibling_path(path: &Path, piece: PieceIndex) -> PathBuf {
    let file_name = path
        .file_name()
        .map_or_else(|| OsString::from("piece"), OsString::from);
    let temp_name = format!(".{}.styx-tmp-{}", file_name.to_string_lossy(), piece.get());
    path.with_file_name(temp_name)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use styx_proto::{FileMode, InfoHashV1, TorrentFile, TorrentInfo, TorrentMetainfo};

    use super::*;
    use crate::{DiskPlan, PieceIndex};

    #[tokio::test]
    async fn commit_piece_writes_and_reads_single_file_piece() {
        let temp = tempfile::tempdir().unwrap();
        let plan = plan(
            temp.path(),
            TorrentInfo {
                name: Bytes::from_static(b"file.bin"),
                piece_length: 16 * 1024,
                pieces: Bytes::from(vec![0_u8; 20]),
                private: false,
                mode: FileMode::Single { length: 5 },
            },
        );
        let store = DiskStore::new(plan);

        store
            .commit_piece(PieceIndex::new(0), Bytes::from_static(b"hello"))
            .await
            .unwrap();

        assert_eq!(
            tokio::fs::read(temp.path().join("file.bin")).await.unwrap(),
            b"hello"
        );
        assert_eq!(
            store.read_piece(PieceIndex::new(0)).await.unwrap(),
            Bytes::from_static(b"hello")
        );
    }

    #[tokio::test]
    async fn commit_piece_splits_multi_file_piece_at_span_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let plan = plan(
            temp.path(),
            TorrentInfo {
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
            },
        );
        let store = DiskStore::new(plan);
        let mut piece = vec![b'a'; 10 * 1024];
        piece.extend(vec![b'b'; 6 * 1024]);

        store
            .commit_piece(PieceIndex::new(0), Bytes::from(piece))
            .await
            .unwrap();

        assert_eq!(
            tokio::fs::read(temp.path().join("album/a.bin"))
                .await
                .unwrap(),
            vec![b'a'; 10 * 1024]
        );
        assert_eq!(
            tokio::fs::read(temp.path().join("album/b.bin"))
                .await
                .unwrap(),
            vec![b'b'; 6 * 1024]
        );
    }

    fn plan(root: &std::path::Path, info: TorrentInfo) -> DiskPlan {
        let meta = TorrentMetainfo {
            announce: None,
            announce_list: Vec::new(),
            url_list: Vec::new(),
            info,
            info_hash_v1: InfoHashV1::new([0; 20]),
            raw_info: Bytes::new(),
        };
        DiskPlan::from_metainfo(&meta, root).unwrap()
    }
}
