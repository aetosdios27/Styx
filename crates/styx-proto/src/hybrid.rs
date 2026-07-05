use crate::file_tree::{FileTreeError, V2FlatFile};
use crate::metainfo::{FileMode, TorrentFile, TorrentMetainfo};
use crate::TorrentInfo;

/// Check if a torrent is hybrid (contains both v1 and v2 metadata).
pub fn is_hybrid(metainfo: &TorrentMetainfo) -> bool {
    metainfo.info.pieces.is_some() && metainfo.info.file_tree.is_some()
}

/// Validate hybrid torrent consistency:
/// - Both v1 and v2 describe the same files in the same order
/// - File lengths match between v1 `files`/`length` and v2 `file tree`
pub fn validate_hybrid_consistency(
    info: &TorrentInfo,
    v2_files: &[V2FlatFile],
) -> Result<(), HybridError> {
    let v1_files = extract_v1_files(info);

    if v1_files.len() != v2_files.len() {
        return Err(HybridError::FileCountMismatch {
            v1: v1_files.len(),
            v2: v2_files.len(),
        });
    }

    for (i, (v1f, v2f)) in v1_files.iter().zip(v2_files.iter()).enumerate() {
        if v1f.length != v2f.entry.length {
            return Err(HybridError::FileLengthMismatch {
                file_index: i,
                v1: v1f.length,
                v2: v2f.entry.length,
            });
        }
    }

    Ok(())
}

fn extract_v1_files(info: &TorrentInfo) -> Vec<TorrentFile> {
    match &info.mode {
        FileMode::Single { length } => vec![TorrentFile {
            length: *length,
            path: vec![info.name.clone()],
        }],
        FileMode::Multi { files } => files.clone(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HybridError {
    #[error("file count mismatch: v1={v1}, v2={v2}")]
    FileCountMismatch { v1: usize, v2: usize },

    #[error("file {file_index} length mismatch: v1={v1}, v2={v2}")]
    FileLengthMismatch { file_index: usize, v1: u64, v2: u64 },

    #[error("file tree error: {0}")]
    FileTreeError(#[from] FileTreeError),
}
