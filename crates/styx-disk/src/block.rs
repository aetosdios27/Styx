use std::collections::BTreeMap;

use bytes::{Bytes, BytesMut};

use crate::{
    BlockLength, BlockOffset, BlockSpec, DiskError, PieceCompletion, PieceIndex, STANDARD_BLOCK_LEN,
};

/// Return the standard request blocks for one piece length.
///
/// # Errors
///
/// Returns [`DiskError::InvalidPieceLength`] if `piece_length` is zero.
pub fn block_specs_for_piece(
    piece: PieceIndex,
    piece_length: u32,
) -> Result<Vec<BlockSpec>, DiskError> {
    if piece_length == 0 {
        return Err(DiskError::InvalidPieceLength);
    }

    let mut specs = Vec::new();
    let mut offset = 0_u32;
    while offset < piece_length {
        let remaining = piece_length
            .checked_sub(offset)
            .ok_or(DiskError::IntegerOverflow)?;
        let length = remaining.min(STANDARD_BLOCK_LEN);
        specs.push(BlockSpec::new(
            piece,
            BlockOffset::new(offset),
            BlockLength::new(length)?,
            piece_length,
        )?);
        offset = offset
            .checked_add(length)
            .ok_or(DiskError::IntegerOverflow)?;
    }

    Ok(specs)
}

/// In-memory block assembly state for one piece.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PieceBuffer {
    piece: PieceIndex,
    piece_length: u32,
    blocks: BTreeMap<u32, Bytes>,
    received_bytes: u32,
}

impl PieceBuffer {
    /// Construct an empty buffer for a piece.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError::InvalidPieceLength`] when `piece_length` is zero.
    pub fn new(piece: PieceIndex, piece_length: u32) -> Result<Self, DiskError> {
        if piece_length == 0 {
            return Err(DiskError::InvalidPieceLength);
        }
        Ok(Self {
            piece,
            piece_length,
            blocks: BTreeMap::new(),
            received_bytes: 0,
        })
    }

    /// Accept a block payload into this buffer.
    ///
    /// # Errors
    ///
    /// Returns a [`DiskError`] when the block is for another piece, has the
    /// wrong payload length, overlaps an existing block, or is out of bounds.
    pub fn accept(
        &mut self,
        spec: BlockSpec,
        payload: Bytes,
    ) -> Result<PieceCompletion, DiskError> {
        if spec.piece() != self.piece {
            return Err(DiskError::InvalidPieceIndex {
                piece: spec.piece().get(),
                piece_count: self.piece.get().saturating_add(1),
            });
        }
        let payload_length =
            u32::try_from(payload.len()).map_err(|_| DiskError::IntegerOverflow)?;
        if payload_length != spec.length().get() {
            return Err(DiskError::InvalidBlockLength {
                length: payload_length,
            });
        }
        BlockSpec::new(self.piece, spec.offset(), spec.length(), self.piece_length)?;
        self.ensure_no_overlap(spec)?;

        self.received_bytes = self
            .received_bytes
            .checked_add(payload_length)
            .ok_or(DiskError::IntegerOverflow)?;
        self.blocks.insert(spec.offset().get(), payload);

        if self.is_complete() {
            Ok(PieceCompletion::Complete { piece: self.piece })
        } else {
            Ok(PieceCompletion::Incomplete)
        }
    }

    /// Return the assembled piece bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError::MissingBlock`] when the piece is incomplete.
    pub fn piece_bytes(&self) -> Result<Bytes, DiskError> {
        if !self.is_complete() {
            return Err(DiskError::MissingBlock);
        }

        let mut output = BytesMut::with_capacity(self.piece_length as usize);
        let mut expected_offset = 0_u32;
        for (offset, payload) in &self.blocks {
            if *offset != expected_offset {
                return Err(DiskError::MissingBlock);
            }
            output.extend_from_slice(payload);
            expected_offset = expected_offset
                .checked_add(u32::try_from(payload.len()).map_err(|_| DiskError::IntegerOverflow)?)
                .ok_or(DiskError::IntegerOverflow)?;
        }
        if expected_offset != self.piece_length {
            return Err(DiskError::MissingBlock);
        }
        Ok(output.freeze())
    }

    /// Return whether the buffer contains every byte in the piece.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.received_bytes == self.piece_length && self.has_contiguous_blocks()
    }

    fn ensure_no_overlap(&self, spec: BlockSpec) -> Result<(), DiskError> {
        let start = spec.offset().get();
        let end = start
            .checked_add(spec.length().get())
            .ok_or(DiskError::IntegerOverflow)?;
        for (existing_start, payload) in &self.blocks {
            let existing_length =
                u32::try_from(payload.len()).map_err(|_| DiskError::IntegerOverflow)?;
            let existing_end = existing_start
                .checked_add(existing_length)
                .ok_or(DiskError::IntegerOverflow)?;
            if start < existing_end && *existing_start < end {
                return Err(DiskError::DuplicateBlock);
            }
        }
        Ok(())
    }

    fn has_contiguous_blocks(&self) -> bool {
        let mut expected_offset = 0_u32;
        for (offset, payload) in &self.blocks {
            if *offset != expected_offset {
                return false;
            }
            let Ok(payload_length) = u32::try_from(payload.len()) else {
                return false;
            };
            let Some(next_offset) = expected_offset.checked_add(payload_length) else {
                return false;
            };
            expected_offset = next_offset;
        }
        expected_offset == self.piece_length
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::{BlockLength, BlockOffset, BlockSpec, DiskError, PieceCompletion, PieceIndex};

    #[test]
    fn block_specs_split_full_piece_into_standard_blocks() {
        let specs = block_specs_for_piece(PieceIndex::new(0), 64 * 1024).unwrap();

        assert_eq!(specs.len(), 4);
        assert_eq!(specs[3].offset().get(), 48 * 1024);
        assert_eq!(specs[3].length().get(), 16 * 1024);
    }

    #[test]
    fn block_specs_split_final_short_piece() {
        let specs = block_specs_for_piece(PieceIndex::new(1), 20 * 1024).unwrap();

        assert_eq!(
            specs,
            vec![
                BlockSpec::new(
                    PieceIndex::new(1),
                    BlockOffset::new(0),
                    BlockLength::new(16 * 1024).unwrap(),
                    20 * 1024,
                )
                .unwrap(),
                BlockSpec::new(
                    PieceIndex::new(1),
                    BlockOffset::new(16 * 1024),
                    BlockLength::new(4 * 1024).unwrap(),
                    20 * 1024,
                )
                .unwrap(),
            ]
        );
    }

    #[test]
    fn piece_buffer_rejects_duplicate_block() {
        let piece = PieceIndex::new(0);
        let spec = block(piece, 0, 4, 8);
        let mut buffer = PieceBuffer::new(piece, 8).unwrap();
        buffer.accept(spec, Bytes::from_static(b"abcd")).unwrap();

        let err = buffer
            .accept(spec, Bytes::from_static(b"abcd"))
            .unwrap_err();

        assert_eq!(err, DiskError::DuplicateBlock);
    }

    #[test]
    fn piece_buffer_rejects_wrong_payload_length() {
        let piece = PieceIndex::new(0);
        let spec = block(piece, 0, 4, 8);
        let mut buffer = PieceBuffer::new(piece, 8).unwrap();

        let err = buffer.accept(spec, Bytes::from_static(b"abc")).unwrap_err();

        assert_eq!(err, DiskError::InvalidBlockLength { length: 3 });
    }

    #[test]
    fn piece_buffer_rejects_block_past_piece_boundary() {
        let piece = PieceIndex::new(0);
        let spec = BlockSpec::new(piece, BlockOffset::new(7), BlockLength::new(2).unwrap(), 8);

        assert_eq!(spec.unwrap_err(), DiskError::BlockOutOfBounds);
    }

    #[test]
    fn piece_buffer_assembles_out_of_order_blocks() {
        let piece = PieceIndex::new(0);
        let mut buffer = PieceBuffer::new(piece, 8).unwrap();
        let last = block(piece, 4, 4, 8);
        let first = block(piece, 0, 4, 8);

        assert_eq!(
            buffer.accept(last, Bytes::from_static(b"efgh")).unwrap(),
            PieceCompletion::Incomplete
        );
        assert_eq!(
            buffer.accept(first, Bytes::from_static(b"abcd")).unwrap(),
            PieceCompletion::Complete { piece }
        );
        assert_eq!(
            buffer.piece_bytes().unwrap(),
            Bytes::from_static(b"abcdefgh")
        );
    }

    fn block(piece: PieceIndex, offset: u32, length: u32, piece_length: u32) -> BlockSpec {
        BlockSpec::new(
            piece,
            BlockOffset::new(offset),
            BlockLength::new(length).unwrap(),
            piece_length,
        )
        .unwrap()
    }
}
