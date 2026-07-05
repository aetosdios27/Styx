use std::time::Duration;

use tokio::sync::mpsc;

use crate::{
    download::download_all_pieces_from_web_seed, RuntimeCommand, RuntimeConfig, RuntimeEngine,
    TorrentCommand, TorrentId, TorrentPlan,
};

#[derive(Debug)]
pub(crate) enum BgEvent {
    Progress {
        id: TorrentId,
        verified_bytes: u64,
        #[allow(dead_code)]
        total_bytes: u64,
    },
    SourceFailed {
        id: TorrentId,
        source: String,
        reason: String,
    },
    Completed {
        id: TorrentId,
    },
    Failed {
        id: TorrentId,
        reason: String,
    },
}

pub(crate) fn spawn_bg_download(
    plan: TorrentPlan,
    tx: mpsc::UnboundedSender<BgEvent>,
    piece_timeout: Duration,
) -> Option<tokio::task::JoinHandle<()>> {
    if tokio::runtime::Handle::try_current().is_err() {
        return None;
    }
    Some(tokio::spawn(async move {
        run_bg_download(plan, tx, piece_timeout).await;
    }))
}

async fn run_bg_download(
    plan: TorrentPlan,
    tx: mpsc::UnboundedSender<BgEvent>,
    piece_timeout: Duration,
) {
    let id = plan.id;
    let total_size = plan.total_size;
    let client = reqwest::Client::new();

    let _ = tx.send(BgEvent::Progress {
        id,
        verified_bytes: 0,
        total_bytes: total_size,
    });

    for seed in &plan.web_seed_urls {
        match download_all_pieces_from_web_seed(&client, &plan, seed, piece_timeout).await {
            Ok(pieces) => {
                let mut engine = match RuntimeEngine::new(RuntimeConfig::default()) {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = tx.send(BgEvent::Failed {
                            id,
                            reason: e.to_string(),
                        });
                        return;
                    }
                };
                if let Err(e) = engine.apply(RuntimeCommand::AddPlan(Box::new(plan.clone()))) {
                    let _ = tx.send(BgEvent::Failed {
                        id,
                        reason: e.to_string(),
                    });
                    return;
                }
                if let Err(e) = engine.apply(RuntimeCommand::Torrent(id, TorrentCommand::Start)) {
                    let _ = tx.send(BgEvent::Failed {
                        id,
                        reason: e.to_string(),
                    });
                    return;
                }

                match engine
                    .complete_from_source_piece_bytes(id, seed.as_str(), pieces)
                    .await
                {
                    Ok(()) => {
                        let _ = tx.send(BgEvent::Completed { id });
                        return;
                    }
                    Err(e) => {
                        let _ = tx.send(BgEvent::SourceFailed {
                            id,
                            source: seed.to_string(),
                            reason: e.to_string(),
                        });
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(BgEvent::SourceFailed {
                    id,
                    source: seed.to_string(),
                    reason: e.to_string(),
                });
            }
        }
    }

    let _ = tx.send(BgEvent::Failed {
        id,
        reason: "all web seeds failed".to_owned(),
    });
}
