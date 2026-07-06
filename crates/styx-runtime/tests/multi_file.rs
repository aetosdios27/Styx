use std::collections::BTreeMap;

use bytes::Bytes;
use sha1::{Digest, Sha1};
use styx_disk::{BlockLength, BlockOffset, BlockSpec, PieceIndex};
use styx_proto::{encode, BencodeValue};
use styx_runtime::{RuntimeCommand, RuntimeConfig, RuntimeEngine, RuntimeEvent, TorrentCommand};

#[tokio::test]
async fn multi_file_torrent_accepts_piece_spanning_file_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("multi.torrent");
    std::fs::write(&torrent, multi_file_torrent_bytes()).unwrap();
    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let id = plan.id;

    assert_eq!(plan.total_size, 24);
    assert_eq!(plan.piece_count(), 2);

    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();
    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))
        .unwrap();

    engine
        .accept_piece_blocks(
            id,
            PieceIndex::new(0),
            vec![(block(0, 0, 16, 16), Bytes::from_static(b"abcdefghijklmnop"))],
        )
        .await
        .unwrap();

    let snapshot = engine.snapshot();
    assert_eq!(snapshot.torrents[0].verified_bytes, 16);
    assert!(engine
        .drain_events()
        .iter()
        .any(|event| matches!(event, RuntimeEvent::PieceVerified { piece: 0, .. })));

    let dl = temp.path().join("downloads");
    assert_eq!(
        tokio::fs::read(dl.join("album/subdir/a.bin"))
            .await
            .unwrap(),
        b"abcdefghijklmnop"
    );
}

#[tokio::test]
async fn multi_file_torrent_routes_pieces_across_multiple_files() {
    let temp = tempfile::tempdir().unwrap();
    let torrent = temp.path().join("multi.torrent");
    std::fs::write(&torrent, multi_file_torrent_bytes()).unwrap();
    let plan =
        styx_runtime::TorrentPlan::from_file(&torrent, temp.path().join("downloads")).unwrap();
    let id = plan.id;

    let mut engine = RuntimeEngine::new(RuntimeConfig::default()).unwrap();
    engine
        .apply(RuntimeCommand::AddPlan(Box::new(plan)))
        .unwrap();
    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Start))
        .unwrap();

    engine
        .accept_piece_blocks(
            id,
            PieceIndex::new(0),
            vec![(block(0, 0, 16, 16), Bytes::from_static(b"abcdefghijklmnop"))],
        )
        .await
        .unwrap();

    engine
        .accept_piece_blocks(
            id,
            PieceIndex::new(1),
            vec![(block(1, 0, 8, 16), Bytes::from_static(b"qrstuvwx"))],
        )
        .await
        .unwrap();

    let snapshot = engine.snapshot();
    assert!(snapshot.torrents[0].verified_bytes >= 8);
    let events = engine.drain_events();
    assert!(events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::PieceVerified { piece: 0, .. })));
    assert!(events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::PieceVerified { piece: 1, .. })));

    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Pause))
        .unwrap();
    assert_eq!(
        engine.snapshot().torrents[0].status,
        styx_runtime::TorrentStatus::Paused
    );

    engine
        .apply(RuntimeCommand::Torrent(id, TorrentCommand::Cancel))
        .unwrap();
    assert_eq!(
        engine.snapshot().torrents[0].status,
        styx_runtime::TorrentStatus::Cancelled
    );
}

fn multi_file_torrent_bytes() -> Vec<u8> {
    let mut top = BTreeMap::new();
    top.insert(
        b"url-list".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"https://mirror.test/")),
    );
    let mut info = BTreeMap::new();
    info.insert(
        b"name".to_vec(),
        BencodeValue::Bytes(Bytes::from_static(b"album")),
    );
    info.insert(b"piece length".to_vec(), BencodeValue::Integer(16));

    let mut pieces = Vec::new();
    pieces.extend_from_slice(&Sha1::digest(b"abcdefghijklmnop"));
    pieces.extend_from_slice(&Sha1::digest(b"qrstuvwx"));
    info.insert(b"pieces".to_vec(), BencodeValue::Bytes(Bytes::from(pieces)));

    let files = vec![
        BencodeValue::Dict(BTreeMap::from([
            (b"length".to_vec(), BencodeValue::Integer(16)),
            (
                b"path".to_vec(),
                BencodeValue::List(vec![
                    BencodeValue::Bytes(Bytes::from_static(b"subdir")),
                    BencodeValue::Bytes(Bytes::from_static(b"a.bin")),
                ]),
            ),
        ])),
        BencodeValue::Dict(BTreeMap::from([
            (b"length".to_vec(), BencodeValue::Integer(8)),
            (
                b"path".to_vec(),
                BencodeValue::List(vec![BencodeValue::Bytes(Bytes::from_static(b"b.bin"))]),
            ),
        ])),
    ];
    info.insert(b"files".to_vec(), BencodeValue::List(files));
    top.insert(b"info".to_vec(), BencodeValue::Dict(info));
    encode(&BencodeValue::Dict(top))
}

fn block(piece: u32, offset: u32, length: u32, piece_length: u32) -> BlockSpec {
    BlockSpec::new(
        PieceIndex::new(piece),
        BlockOffset::new(offset),
        BlockLength::new(length).unwrap(),
        piece_length,
    )
    .unwrap()
}
