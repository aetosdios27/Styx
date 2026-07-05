//! Disk planning, piece assembly, verification, and storage for Styx.

mod block;
mod error;
mod hash;
mod layout;
mod manager;
mod merkle;
mod storage;
mod types;

pub use block::{block_specs_for_piece, PieceBuffer};
pub use error::DiskError;
pub use hash::{
    merkle_parent_hash, merkle_root, piece_layer_hashes, piece_root, sha256_block_hash,
    verify_block_with_proof, verify_v1_piece, verify_v2_piece_data, MERKLE_BLOCK_SIZE,
};
pub use layout::{DiskPlan, FileEntry, FileSpan};
pub use manager::PieceManager;
pub use storage::DiskStore;
pub use types::{
    BlockLength, BlockOffset, BlockSpec, PieceCompletion, PieceIndex, ResumeSummary,
    VerificationResult, STANDARD_BLOCK_LEN,
};
