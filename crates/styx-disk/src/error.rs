use std::io;

/// Errors returned by disk planning, verification, and storage operations.
#[derive(Debug, thiserror::Error)]
pub enum DiskError {
    /// A torrent or API call supplied an invalid piece length.
    #[error("piece length must be greater than zero")]
    InvalidPieceLength,
    /// A piece index was outside the torrent's piece count.
    #[error("piece index {piece} is outside {piece_count} pieces")]
    InvalidPieceIndex {
        /// Requested piece index.
        piece: u32,
        /// Number of pieces in the torrent.
        piece_count: u32,
    },
    /// The metainfo piece hash count does not match the planned piece count.
    #[error("piece hash count {actual} does not match expected piece count {expected}")]
    InvalidPieceHashCount {
        /// Expected number of piece hashes.
        expected: u32,
        /// Actual number of piece hashes.
        actual: u32,
    },
    /// A block length was zero or otherwise invalid.
    #[error("invalid block length {length}")]
    InvalidBlockLength {
        /// Requested block length.
        length: u32,
    },
    /// A block offset was outside the selected piece.
    #[error("block offset {offset} is outside piece length {piece_length}")]
    InvalidBlockOffset {
        /// Requested block offset.
        offset: u32,
        /// Length of the selected piece.
        piece_length: u32,
    },
    /// A block offset plus length extends past the selected piece.
    #[error("block extends past piece boundary")]
    BlockOutOfBounds,
    /// The same block was submitted more than once.
    #[error("duplicate block")]
    DuplicateBlock,
    /// A piece is incomplete because at least one block is missing.
    #[error("missing block")]
    MissingBlock,
    /// Piece bytes did not match the expected hash.
    #[error("piece hash mismatch")]
    HashMismatch,
    /// A target path was not safe to write under the destination root.
    #[error("unsafe path")]
    UnsafePath,
    /// Checked integer arithmetic overflowed.
    #[error("integer overflow")]
    IntegerOverflow,
    /// Disk IO failed.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl PartialEq for DiskError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::InvalidPieceLength, Self::InvalidPieceLength)
            | (Self::BlockOutOfBounds, Self::BlockOutOfBounds)
            | (Self::DuplicateBlock, Self::DuplicateBlock)
            | (Self::MissingBlock, Self::MissingBlock)
            | (Self::HashMismatch, Self::HashMismatch)
            | (Self::UnsafePath, Self::UnsafePath)
            | (Self::IntegerOverflow, Self::IntegerOverflow) => true,
            (
                Self::InvalidPieceIndex {
                    piece: left_piece,
                    piece_count: left_count,
                },
                Self::InvalidPieceIndex {
                    piece: right_piece,
                    piece_count: right_count,
                },
            ) => left_piece == right_piece && left_count == right_count,
            (
                Self::InvalidPieceHashCount {
                    expected: left_expected,
                    actual: left_actual,
                },
                Self::InvalidPieceHashCount {
                    expected: right_expected,
                    actual: right_actual,
                },
            ) => left_expected == right_expected && left_actual == right_actual,
            (
                Self::InvalidBlockLength {
                    length: left_length,
                },
                Self::InvalidBlockLength {
                    length: right_length,
                },
            ) => left_length == right_length,
            (
                Self::InvalidBlockOffset {
                    offset: left_offset,
                    piece_length: left_piece_length,
                },
                Self::InvalidBlockOffset {
                    offset: right_offset,
                    piece_length: right_piece_length,
                },
            ) => left_offset == right_offset && left_piece_length == right_piece_length,
            (Self::Io(left), Self::Io(right)) => left.kind() == right.kind(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_piece_index_error_is_structured() {
        let err = DiskError::InvalidPieceIndex {
            piece: 3,
            piece_count: 2,
        };

        assert_eq!(err.to_string(), "piece index 3 is outside 2 pieces");
    }
}
