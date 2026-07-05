use std::{fs, path::Path};

use bytes::Bytes;
use styx_disk::{DiskPlan, PieceIndex};
use styx_proto::{decode_torrent, FileMode, InfoHashV1, InfoHashV2, TorrentMetainfo};
use url::Url;

use crate::{RuntimeError, SmokeConfig, SmokeTarget};

#[derive(Clone, Debug)]
pub struct TorrentPlan {
    pub metainfo: TorrentMetainfo,
    pub id: TorrentId,
    pub info_hash: InfoHashV1,
    pub info_hash_v2: Option<InfoHashV2>,
    pub name: String,
    pub total_size: u64,
    pub announce_urls: Vec<Url>,
    pub web_seed_urls: Vec<Url>,
    pub disk_plan: DiskPlan,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TorrentId(InfoHashV1);

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

impl TorrentPlan {
    pub fn from_file(
        torrent_path: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<Self, RuntimeError> {
        let bytes = fs::read(torrent_path)?;
        let metainfo = decode_torrent(&bytes)?;
        Self::from_metainfo(metainfo, destination)
    }

    pub fn from_metainfo(
        metainfo: TorrentMetainfo,
        destination: impl AsRef<Path>,
    ) -> Result<Self, RuntimeError> {
        let total_size = torrent_size(&metainfo);
        let announce_urls = http_announce_urls(&metainfo)?;
        let web_seed_urls = web_seed_urls(&metainfo)?;
        if announce_urls.is_empty() && web_seed_urls.is_empty() {
            return Err(RuntimeError::NoHttpTracker);
        }
        let info_hash_v2 = metainfo.info_hash_v2;
        let disk_plan = if info_hash_v2.is_some() && metainfo.info.pieces.is_none() {
            return Err(RuntimeError::V2NotSupported);
        } else {
            DiskPlan::from_metainfo(&metainfo, destination)?
        };
        let info_hash = metainfo.info_hash_v1;
        Ok(Self {
            id: TorrentId::new(info_hash),
            info_hash,
            info_hash_v2,
            name: String::from_utf8_lossy(&metainfo.info.name).into_owned(),
            metainfo,
            total_size,
            announce_urls,
            web_seed_urls,
            disk_plan,
        })
    }

    #[must_use]
    pub fn piece_count(&self) -> u32 {
        self.disk_plan.piece_count()
    }

    pub fn piece_length(&self, piece: PieceIndex) -> Result<u32, RuntimeError> {
        Ok(self.disk_plan.piece_length(piece)?)
    }
}

impl TorrentId {
    #[must_use]
    pub const fn new(info_hash: InfoHashV1) -> Self {
        Self(info_hash)
    }

    #[must_use]
    pub const fn info_hash(self) -> InfoHashV1 {
        self.0
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 20] {
        self.0.as_bytes()
    }
}

impl Ord for TorrentId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl PartialOrd for TorrentId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub fn load_torrent_plan(
    torrent_path: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    config: &SmokeConfig,
) -> Result<TorrentSmokePlan, RuntimeError> {
    config.validate()?;
    let plan = TorrentPlan::from_file(torrent_path, destination)?;
    let target_piece = match config.target {
        SmokeTarget::FirstPiece => PieceIndex::new(0),
    };
    plan.disk_plan.piece_length(target_piece)?;

    Ok(TorrentSmokePlan {
        info_hash: plan.info_hash,
        metainfo: plan.metainfo,
        total_size: plan.total_size,
        left: plan.total_size,
        announce_urls: plan.announce_urls,
        web_seed_urls: plan.web_seed_urls,
        target_piece,
        disk_plan: plan.disk_plan,
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
