use crate::DiskError;

/// Standard BitTorrent block size used for request decomposition.
pub const STANDARD_BLOCK_LEN: u32 = 16 * 1024;

/// Zero-based torrent piece index.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PieceIndex(u32);

impl PieceIndex {
    /// Construct a piece index from a raw `u32`.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the raw index value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Byte offset of a block within a piece.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlockOffset(u32);

impl BlockOffset {
    /// Construct a block offset from a raw `u32`.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the raw offset value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Length of a block in bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlockLength(u32);

impl BlockLength {
    /// Construct a non-zero block length.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError::InvalidBlockLength`] when `value` is zero.
    pub const fn new(value: u32) -> Result<Self, DiskError> {
        if value == 0 {
            Err(DiskError::InvalidBlockLength { length: value })
        } else {
            Ok(Self(value))
        }
    }

    /// Return the raw length value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A block request or received block identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BlockSpec {
    piece: PieceIndex,
    offset: BlockOffset,
    length: BlockLength,
}

impl BlockSpec {
    /// Construct a block spec after validating it against the piece length.
    ///
    /// # Errors
    ///
    /// Returns a [`DiskError`] when the piece length is zero, the offset is
    /// outside the piece, or the block extends past the piece boundary.
    pub const fn new(
        piece: PieceIndex,
        offset: BlockOffset,
        length: BlockLength,
        piece_length: u32,
    ) -> Result<Self, DiskError> {
        if piece_length == 0 {
            return Err(DiskError::InvalidPieceLength);
        }
        if offset.get() >= piece_length {
            return Err(DiskError::InvalidBlockOffset {
                offset: offset.get(),
                piece_length,
            });
        }
        let Some(end) = offset.get().checked_add(length.get()) else {
            return Err(DiskError::IntegerOverflow);
        };
        if end > piece_length {
            return Err(DiskError::BlockOutOfBounds);
        }
        Ok(Self {
            piece,
            offset,
            length,
        })
    }

    /// Piece this block belongs to.
    #[must_use]
    pub const fn piece(self) -> PieceIndex {
        self.piece
    }

    /// Offset within the piece.
    #[must_use]
    pub const fn offset(self) -> BlockOffset {
        self.offset
    }

    /// Block length.
    #[must_use]
    pub const fn length(self) -> BlockLength {
        self.length
    }
}

/// Result of accepting a block into the in-memory piece buffer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PieceCompletion {
    /// The piece still needs more blocks.
    Incomplete,
    /// All blocks for the piece are present.
    Complete { piece: PieceIndex },
}

/// Result of verifying a piece's bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerificationResult {
    /// The piece matched its expected hash and is durable on disk.
    Verified { piece: PieceIndex },
    /// The piece did not match its expected hash.
    HashMismatch { piece: PieceIndex },
}

/// Summary returned after verifying existing files on startup.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResumeSummary {
    /// Number of pieces verified from existing disk bytes.
    pub verified: u32,
    /// Number of pieces whose files were absent.
    pub missing: u32,
    /// Number of pieces present but failing hash verification.
    pub failed: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_length_rejects_zero() {
        let err = BlockLength::new(0).unwrap_err();

        assert_eq!(err, DiskError::InvalidBlockLength { length: 0 });
    }

    #[test]
    fn block_spec_rejects_end_past_piece_length() {
        let piece = PieceIndex::new(0);
        let offset = BlockOffset::new(15);
        let length = BlockLength::new(2).unwrap();

        let err = BlockSpec::new(piece, offset, length, 16).unwrap_err();

        assert_eq!(err, DiskError::BlockOutOfBounds);
    }

    #[test]
    fn block_spec_accepts_last_byte_of_piece() {
        let piece = PieceIndex::new(7);
        let offset = BlockOffset::new(15);
        let length = BlockLength::new(1).unwrap();

        let spec = BlockSpec::new(piece, offset, length, 16).unwrap();

        assert_eq!(spec.piece(), piece);
    }
}
