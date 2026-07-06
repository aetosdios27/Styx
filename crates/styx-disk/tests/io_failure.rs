use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_disk::{DiskError, DiskPlan, DiskStore, PieceIndex, PieceManager, ResumeSummary};
use styx_proto::{FileMode, InfoHashV1, TorrentInfo, TorrentMetainfo};

#[tokio::test]
async fn write_to_read_only_dir_fails_with_io_error() {
    let temp = tempfile::tempdir().unwrap();
    let read_only = temp.path().join("readonly");
    tokio::fs::create_dir(&read_only).await.unwrap();

    let mut perms = tokio::fs::metadata(&read_only).await.unwrap().permissions();
    perms.set_readonly(true);
    tokio::fs::set_permissions(&read_only, perms).await.unwrap();

    let plan = plan_for_root_and_bytes(&read_only, b"hello");
    let store = DiskStore::new(plan);

    let result = store
        .commit_piece(PieceIndex::new(0), Bytes::from_static(b"hello"))
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, DiskError::Io(ref e) if e.kind() == std::io::ErrorKind::PermissionDenied)
    );
}

#[tokio::test]
async fn corrupted_on_disk_piece_fails_resume_verify() {
    let temp = tempfile::tempdir().unwrap();
    let plan = plan_for_root_and_pieces(temp.path(), &[b"abcd".as_slice(), b"efgh".as_slice()], 4);
    let store = DiskStore::new(plan.clone());

    store
        .commit_piece(PieceIndex::new(0), Bytes::from_static(b"abcd"))
        .await
        .unwrap();
    store
        .commit_piece(PieceIndex::new(1), Bytes::from_static(b"efgh"))
        .await
        .unwrap();

    tokio::fs::write(temp.path().join("file.bin"), b"xxxxyyyy")
        .await
        .unwrap();

    let mut manager = PieceManager::new(plan);
    let summary = manager.resume_verify().await.unwrap();

    assert_eq!(
        summary,
        ResumeSummary {
            verified: 0,
            missing: 0,
            failed: 2,
        }
    );
    assert!(!manager.has_piece(PieceIndex::new(0)));
    assert!(!manager.has_piece(PieceIndex::new(1)));
}

#[tokio::test]
async fn multi_file_piece_commit_splits_across_file_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let plan = multi_file_plan(temp.path());
    let store = DiskStore::new(plan.clone());

    let piece0_content = {
        let mut data = vec![b'a'; 10 * 1024];
        data.extend(vec![b'b'; 6 * 1024]);
        Bytes::from(data)
    };
    store
        .commit_piece(PieceIndex::new(0), piece0_content)
        .await
        .unwrap();

    let mut piece1_content = vec![b'b'; 4 * 1024];
    store
        .commit_piece(
            PieceIndex::new(1),
            Bytes::from({
                piece1_content.resize(4 * 1024, 0);
                piece1_content
            }),
        )
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
        vec![b'b'; 10 * 1024]
    );

    let read_piece = store.read_piece(PieceIndex::new(0)).await.unwrap();
    let mut expected_piece0 = vec![b'a'; 10 * 1024];
    expected_piece0.extend(vec![b'b'; 6 * 1024]);
    assert_eq!(read_piece.to_vec(), expected_piece0);
}

fn plan_for_root_and_bytes(root: &std::path::Path, bytes: &[u8]) -> DiskPlan {
    plan_for_root_and_pieces(root, &[bytes], bytes.len() as u64)
}

fn plan_for_root_and_pieces(
    root: &std::path::Path,
    pieces: &[&[u8]],
    piece_length: u64,
) -> DiskPlan {
    let mut hashes = Vec::with_capacity(pieces.len() * 20);
    let mut total_length = 0_u64;
    for piece in pieces {
        let digest: [u8; 20] = Sha1::digest(piece).into();
        hashes.extend_from_slice(&digest);
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
    DiskPlan::from_metainfo(&meta, root).unwrap()
}

fn multi_file_plan(root: &std::path::Path) -> DiskPlan {
    let piece_length: u64 = 16 * 1024;
    let file_a_bytes: u64 = 10 * 1024;
    let file_b_bytes: u64 = 10 * 1024;
    let total_length = file_a_bytes + file_b_bytes;
    let piece_count = total_length.div_ceil(piece_length) as usize;
    let mut hashes = Vec::with_capacity(piece_count * 20);
    for _ in 0..piece_count {
        hashes.extend_from_slice(&[0u8; 20]);
    }
    let meta = TorrentMetainfo {
        announce: None,
        announce_list: Vec::new(),
        url_list: Vec::new(),
        info: TorrentInfo {
            name: Bytes::from_static(b"album"),
            piece_length,
            pieces: Some(Bytes::from(hashes)),
            private: false,
            mode: FileMode::Multi {
                files: vec![
                    styx_proto::TorrentFile {
                        length: file_a_bytes,
                        path: vec![Bytes::from_static(b"a.bin")],
                    },
                    styx_proto::TorrentFile {
                        length: file_b_bytes,
                        path: vec![Bytes::from_static(b"b.bin")],
                    },
                ],
            },
            meta_version: None,
            file_tree: None,
        },
        info_hash_v1: InfoHashV1::new([0; 20]),
        info_hash_v2: None,
        piece_layers: None,
        raw_info: Bytes::new(),
    };
    DiskPlan::from_metainfo(&meta, root).unwrap()
}
