#![allow(dead_code)]

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_disk::{DiskPlan, DiskStore, PieceIndex, PieceManager};
use styx_proto::{FileMode, InfoHashV1, TorrentInfo, TorrentMetainfo};

fn plan_and_store_for_pieces(root: &std::path::Path, pieces: &[&[u8]]) -> (DiskPlan, DiskStore) {
    let piece_length = pieces.first().map(|p| p.len() as u64).unwrap_or(0);
    let mut hashes = Vec::with_capacity(pieces.len() * 20);
    let mut total_length = 0_u64;
    for piece in pieces {
        hashes.extend_from_slice(&Sha1::digest(piece));
        total_length += piece.len() as u64;
    }
    let meta = TorrentMetainfo {
        announce: None,
        announce_list: Vec::new(),
        url_list: Vec::new(),
        info: TorrentInfo {
            name: Bytes::from_static(b"file.bin"),
            piece_length,
            pieces: Some(Bytes::from(hashes)),
            private: false,
            mode: FileMode::Single {
                length: total_length,
            },
            meta_version: None,
            file_tree: None,
        },
        info_hash_v1: InfoHashV1::new([0; 20]),
        info_hash_v2: None,
        piece_layers: None,
        raw_info: Bytes::new(),
    };
    let plan = DiskPlan::from_metainfo(&meta, root).unwrap();
    let store = DiskStore::new(plan.clone());
    (plan, store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resume_torture_deleted_piece_re_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let piece0_data = [0xABu8; 16384];
        let piece1_data = [0xCDu8; 16384];
        let pieces: [&[u8]; 2] = [&piece0_data, &piece1_data];

        let (plan, store) = plan_and_store_for_pieces(dir.path(), &pieces);

        store
            .commit_piece(PieceIndex::new(0), Bytes::copy_from_slice(&piece0_data))
            .await
            .unwrap();
        store
            .commit_piece(PieceIndex::new(1), Bytes::copy_from_slice(&piece1_data))
            .await
            .unwrap();

        let file_path = dir.path().join("file.bin");
        let mut on_disk = tokio::fs::read(&file_path).await.unwrap();
        on_disk[..piece0_data.len()].fill(0);
        tokio::fs::write(&file_path, &on_disk).await.unwrap();

        let mut manager = PieceManager::new(plan);
        let summary = manager.resume_verify().await.unwrap();

        assert_eq!(summary.verified, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.missing, 0);
        assert!(!manager.has_piece(PieceIndex::new(0)));
        assert!(manager.has_piece(PieceIndex::new(1)));
    }

    #[tokio::test]
    async fn resume_torture_partial_piece_write_marked_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let piece_data = [0xABu8; 16384];

        let (plan, store) = plan_and_store_for_pieces(dir.path(), &[&piece_data]);

        store
            .commit_piece(PieceIndex::new(0), Bytes::copy_from_slice(&piece_data))
            .await
            .unwrap();

        let file_path = dir.path().join("file.bin");
        tokio::fs::write(&file_path, b"").await.unwrap();

        let mut manager = PieceManager::new(plan);
        let summary = manager.resume_verify().await.unwrap();

        assert_eq!(summary.verified, 0);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.missing, 1);
        assert!(!manager.has_piece(PieceIndex::new(0)));
    }

    #[tokio::test]
    async fn resume_torture_zero_length_piece_file_handled_gracefully() {
        let dir = tempfile::tempdir().unwrap();
        let piece_data = [0xABu8; 16384];

        let (plan, store) = plan_and_store_for_pieces(dir.path(), &[&piece_data]);

        store
            .commit_piece(PieceIndex::new(0), Bytes::copy_from_slice(&piece_data))
            .await
            .unwrap();

        let file_path = dir.path().join("file.bin");
        tokio::fs::write(&file_path, vec![0u8; piece_data.len()])
            .await
            .unwrap();

        let mut manager = PieceManager::new(plan);
        let summary = manager.resume_verify().await.unwrap();

        assert_eq!(summary.failed, 1);
        assert_eq!(summary.verified, 0);
        assert!(!manager.has_piece(PieceIndex::new(0)));
    }

    #[tokio::test]
    async fn resume_torture_swap_pieces_between_torrents_fails_both() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();

        let piece_a_data = [0xABu8; 16384];
        let piece_b_data = [0xCDu8; 16384];

        let (plan_a, store_a) = plan_and_store_for_pieces(dir_a.path(), &[&piece_a_data]);
        let (plan_b, store_b) = plan_and_store_for_pieces(dir_b.path(), &[&piece_b_data]);

        store_a
            .commit_piece(PieceIndex::new(0), Bytes::copy_from_slice(&piece_a_data))
            .await
            .unwrap();
        store_b
            .commit_piece(PieceIndex::new(0), Bytes::copy_from_slice(&piece_b_data))
            .await
            .unwrap();

        let file_a = dir_a.path().join("file.bin");
        let file_b = dir_b.path().join("file.bin");
        let a_content = tokio::fs::read(&file_a).await.unwrap();
        let b_content = tokio::fs::read(&file_b).await.unwrap();
        tokio::fs::write(&file_a, &b_content).await.unwrap();
        tokio::fs::write(&file_b, &a_content).await.unwrap();

        let mut manager_a = PieceManager::new(plan_a);
        let mut manager_b = PieceManager::new(plan_b);
        let summary_a = manager_a.resume_verify().await.unwrap();
        let summary_b = manager_b.resume_verify().await.unwrap();

        assert_eq!(summary_a.verified, 0);
        assert_eq!(summary_b.verified, 0);
        assert!(!manager_a.has_piece(PieceIndex::new(0)));
        assert!(!manager_b.has_piece(PieceIndex::new(0)));
    }
}
