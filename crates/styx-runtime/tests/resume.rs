use std::collections::BTreeMap;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_proto::{encode, BencodeValue};
use styx_runtime::{RuntimeCommand, RuntimeConfig, RuntimeEngine, TorrentPlan, TorrentStatus};

#[tokio::test]
async fn runtime_engine_rechecks_existing_pieces_before_resume_completion() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    let destination = temp.path().join("downloads");
    std::fs::write(
        &torrent,
        torrent_from_chunks(&[b"abcd".as_slice(), b"efgh".as_slice(), b"ij".as_slice()]),
    )
    .unwrap();

    let plan = TorrentPlan::from_file(&torrent, &destination).unwrap();
    let id = plan.id;
    let mut first = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    first
        .apply(RuntimeCommand::AddPlan(Box::new(plan.clone())))
        .unwrap();
    first
        .complete_from_piece_bytes(
            id,
            vec![
                Bytes::from_static(b"abcd"),
                Bytes::from_static(b"efgh"),
                Bytes::from_static(b"ij"),
            ],
        )
        .await
        .unwrap();

    tokio::fs::write(destination.join("file.bin"), b"abcdXXXX")
        .await
        .unwrap();

    let mut resumed = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    resumed
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();
    let summary = resumed.resume_verify(id).await.unwrap();

    assert_eq!(summary.verified, 1);
    assert_eq!(resumed.snapshot().torrents[0].verified_bytes, 4);

    resumed
        .complete_from_piece_bytes(
            id,
            vec![
                Bytes::from_static(b"abcd"),
                Bytes::from_static(b"efgh"),
                Bytes::from_static(b"ij"),
            ],
        )
        .await
        .unwrap();

    assert_eq!(
        resumed.snapshot().torrents[0].status,
        TorrentStatus::Complete
    );
    assert_eq!(
        tokio::fs::read(destination.join("file.bin")).await.unwrap(),
        b"abcdefghij"
    );
}

fn torrent_from_chunks(chunks: &[&[u8]]) -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"url-list".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"https://mirror.test/")),
    );
    let mut info = BTreeMap::new();
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"file.bin")),
    );
    info.insert(b"piece length".to_vec(), BencodeValue::Integer(4));
    info.insert(
        b"length".to_vec(),
        BencodeValue::Integer(chunks.iter().map(|chunk| chunk.len() as i64).sum()),
    );
    let mut pieces = Vec::new();
    for chunk in chunks {
        pieces.extend_from_slice(&Sha1::digest(chunk));
    }
    info.insert(b"pieces".to_vec(), BencodeValue::Bytes(Bytes::from(pieces)));
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}
