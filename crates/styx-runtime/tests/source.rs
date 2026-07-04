use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use url::Url;

use styx_runtime::{
    RuntimeConfig, SourceCandidate, SourceFailure, SourceKind, SourceState, SourceTable,
};

#[test]
fn source_table_deduplicates_peer_and_web_seed_candidates() {
    let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 6881);
    let web_seed = Url::parse("https://mirror.test/file.iso").unwrap();

    let table = SourceTable::from_candidates(
        vec![
            SourceCandidate::peer(peer),
            SourceCandidate::peer(peer),
            SourceCandidate::web_seed(web_seed.clone()),
            SourceCandidate::web_seed(web_seed),
        ],
        &RuntimeConfig::default(),
    )
    .unwrap();

    assert_eq!(table.len(), 2);
    assert_eq!(table.by_kind(SourceKind::Peer).count(), 1);
    assert_eq!(table.by_kind(SourceKind::WebSeed).count(), 1);
}

#[test]
fn source_table_moves_retryable_failures_to_exhausted_after_budget() {
    let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 6881);
    let mut table =
        SourceTable::from_candidates(vec![SourceCandidate::peer(peer)], &RuntimeConfig::default())
            .unwrap();
    let id = table.next_candidates(1)[0].id;

    for _ in 0..RuntimeConfig::default().limits.source_retry_limit {
        table.record_failure(id, SourceFailure::Timeout).unwrap();
    }

    assert_eq!(table.state(id).unwrap(), SourceState::Exhausted);
    assert!(table.next_candidates(1).is_empty());
}

#[test]
fn source_table_quarantines_corrupt_sources_without_retry() {
    let web_seed = Url::parse("https://mirror.test/file.iso").unwrap();
    let mut table = SourceTable::from_candidates(
        vec![SourceCandidate::web_seed(web_seed)],
        &RuntimeConfig::default(),
    )
    .unwrap();
    let id = table.next_candidates(1)[0].id;

    table
        .record_failure(id, SourceFailure::CorruptData)
        .unwrap();

    assert_eq!(table.state(id).unwrap(), SourceState::Quarantined);
    assert!(table.next_candidates(1).is_empty());
}
