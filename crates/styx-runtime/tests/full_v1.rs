use std::collections::BTreeMap;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_proto::{encode, BencodeValue};
use styx_runtime::{
    RuntimeCommand, RuntimeConfig, RuntimeEngine, RuntimeError, RuntimeEvent, TorrentPlan,
    TorrentStatus,
};

#[tokio::test]
async fn runtime_engine_completes_three_piece_v1_torrent_from_piece_bytes() {
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
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();

    engine
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

    let snapshot = engine.snapshot();
    assert_eq!(snapshot.torrents[0].status, TorrentStatus::Complete);
    assert_eq!(snapshot.torrents[0].verified_bytes, 10);
    assert_eq!(snapshot.torrents[0].progress(), 1.0);
    assert_eq!(
        tokio::fs::read(destination.join("file.bin")).await.unwrap(),
        b"abcdefghij"
    );
    assert!(engine
        .drain_events()
        .iter()
        .any(|event| matches!(event, RuntimeEvent::TaskCompleted { torrent } if *torrent == id)));
}

#[tokio::test]
async fn runtime_engine_quarantines_corrupt_source_then_completes_from_good_source() {
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
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();

    let bad = engine
        .complete_from_source_piece_bytes(
            id,
            "webseed:bad",
            vec![
                Bytes::from_static(b"abcd"),
                Bytes::from_static(b"xxxx"),
                Bytes::from_static(b"ij"),
            ],
        )
        .await
        .unwrap_err();

    assert_eq!(bad.retry_class(), styx_runtime::RetryClass::Quarantine);
    assert_eq!(engine.snapshot().torrents[0].verified_bytes, 4);

    engine
        .complete_from_source_piece_bytes(
            id,
            "webseed:good",
            vec![
                Bytes::from_static(b"abcd"),
                Bytes::from_static(b"efgh"),
                Bytes::from_static(b"ij"),
            ],
        )
        .await
        .unwrap();

    assert_eq!(
        engine.snapshot().torrents[0].status,
        TorrentStatus::Complete
    );
    assert!(engine
        .drain_events()
        .iter()
        .any(|event| matches!(event, RuntimeEvent::SourceQuarantined { source, .. } if source == "webseed:bad")));
}

#[tokio::test]
async fn runtime_engine_reports_terminal_failure_when_all_sources_fail() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("sample.torrent");
    std::fs::write(
        &torrent,
        torrent_from_chunks(&[b"abcd".as_slice(), b"efgh".as_slice(), b"ij".as_slice()]),
    )
    .unwrap();
    let plan = TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let id = plan.id;
    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();

    let err = engine
        .complete_from_sources(
            id,
            vec![
                (
                    "webseed:bad-a",
                    vec![
                        Bytes::from_static(b"xxxx"),
                        Bytes::from_static(b"yyyy"),
                        Bytes::from_static(b"zz"),
                    ],
                ),
                (
                    "webseed:bad-b",
                    vec![
                        Bytes::from_static(b"1111"),
                        Bytes::from_static(b"2222"),
                        Bytes::from_static(b"33"),
                    ],
                ),
            ],
        )
        .await
        .unwrap_err();

    let RuntimeError::AllPeersFailed { last_error } = err else {
        panic!("expected all peers failed error");
    };
    assert!(last_error.contains("webseed:bad-b"));
    assert!(last_error.contains("piece 0 failed hash verification"));
    assert_eq!(engine.snapshot().torrents[0].status, TorrentStatus::Failed);
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
