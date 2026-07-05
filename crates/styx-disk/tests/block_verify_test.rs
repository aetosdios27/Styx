use styx_disk::{DiskPlan, PieceIndex, PieceManager};
use styx_proto::V2FlatFile;

fn make_v2_plan(root: &std::path::Path) -> DiskPlan {
    let data = vec![0xABu8; 32768];
    let block0_hash = styx_disk::sha256_block_hash(&data[0..16384]);
    let block1_hash = styx_disk::sha256_block_hash(&data[16384..32768]);
    let piece_hash = styx_disk::piece_root(&[block0_hash, block1_hash], 2);

    let flat_file = V2FlatFile {
        path_components: vec![b"test.bin".to_vec()],
        entry: styx_proto::V2FileEntry {
            length: 32768,
            pieces_root: None,
        },
    };

    DiskPlan::new_v2(root, &[flat_file], 32768, vec![piece_hash]).unwrap()
}

#[tokio::test]
async fn piece_manager_verify_block_valid() {
    let temp = tempfile::tempdir().unwrap();
    let plan = make_v2_plan(temp.path());
    let manager = PieceManager::new(plan);

    let block_data = vec![0xABu8; 16384];
    let block1_hash = styx_disk::sha256_block_hash(&[0xABu8; 16384]);
    let proof = vec![block1_hash];

    let result = manager
        .verify_block(PieceIndex::new(0), 0, &block_data, &proof)
        .unwrap();
    assert!(result);
}
