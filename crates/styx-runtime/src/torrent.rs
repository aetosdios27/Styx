use std::{fs, path::Path};

use bytes::Bytes;
use styx_disk::{DiskPlan, PieceIndex};
use styx_proto::{decode_torrent, FileMode, InfoHashV1, TorrentMetainfo};
use url::Url;

use crate::{RuntimeError, SmokeConfig, SmokeTarget};

#[derive(Clone, Debug)]
pub struct TorrentSmokePlan {
    pub metainfo: TorrentMetainfo,
    pub info_hash: InfoHashV1,
    pub total_size: u64,
    pub left: u64,
    pub announce_urls: Vec<Url>,
    pub web_seed_urls: Vec<Url>,
    pub target_piece: PieceIndex,
    pub disk_plan: DiskPlan,
}

pub fn load_torrent_plan(
    torrent_path: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    config: &SmokeConfig,
) -> Result<TorrentSmokePlan, RuntimeError> {
    config.validate()?;
    let bytes = fs::read(torrent_path)?;
    let metainfo = decode_torrent(&bytes)?;
    let total_size = torrent_size(&metainfo);
    let announce_urls = http_announce_urls(&metainfo)?;
    let web_seed_urls = web_seed_urls(&metainfo)?;
    if announce_urls.is_empty() && web_seed_urls.is_empty() {
        return Err(RuntimeError::NoHttpTracker);
    }
    let disk_plan = DiskPlan::from_metainfo(&metainfo, destination)?;
    let target_piece = match config.target {
        SmokeTarget::FirstPiece => PieceIndex::new(0),
    };
    disk_plan.piece_length(target_piece)?;

    Ok(TorrentSmokePlan {
        info_hash: metainfo.info_hash_v1,
        metainfo,
        total_size,
        left: total_size,
        announce_urls,
        web_seed_urls,
        target_piece,
        disk_plan,
    })
}

fn torrent_size(meta: &TorrentMetainfo) -> u64 {
    match &meta.info.mode {
        FileMode::Single { length } => *length,
        FileMode::Multi { files } => files.iter().map(|file| file.length).sum(),
    }
}

fn http_announce_urls(meta: &TorrentMetainfo) -> Result<Vec<Url>, RuntimeError> {
    let mut urls = Vec::new();
    if let Some(announce) = &meta.announce {
        push_http_url(&mut urls, announce)?;
    }
    for tier in &meta.announce_list {
        for announce in tier {
            push_http_url(&mut urls, announce)?;
        }
    }
    Ok(urls)
}

fn web_seed_urls(meta: &TorrentMetainfo) -> Result<Vec<Url>, RuntimeError> {
    let mut urls = Vec::new();
    for seed in &meta.url_list {
        push_http_url(&mut urls, seed)?;
    }
    Ok(urls)
}

fn push_http_url(urls: &mut Vec<Url>, bytes: &Bytes) -> Result<(), RuntimeError> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(());
    };
    let Ok(url) = Url::parse(text) else {
        return Err(RuntimeError::InvalidTrackerUrl {
            url: text.to_owned(),
        });
    };
    if matches!(url.scheme(), "http" | "https") && !urls.iter().any(|known| known == &url) {
        urls.push(url);
    }
    Ok(())
}
