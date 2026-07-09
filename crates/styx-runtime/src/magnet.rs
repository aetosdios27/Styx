use std::path::PathBuf;

use styx_proto::{decode_torrent, parse_magnet_uri, InfoHashV1, MagnetUri, PeerId};

use crate::{fetch_metadata_from_peer, MetadataFetchConfig, RuntimeError, TorrentId, TorrentPlan};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MagnetAdd {
    pub uri: String,
    pub destination: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ResolvedMagnet {
    pub magnet: MagnetUri,
    pub plan: TorrentPlan,
}

pub async fn resolve_magnet_from_exact_peers(
    add: MagnetAdd,
    peer_id: PeerId,
    config: MetadataFetchConfig,
) -> Result<ResolvedMagnet, RuntimeError> {
    let magnet = parse_magnet_uri(&add.uri).map_err(|err| RuntimeError::Magnet(err.to_string()))?;
    let peers = magnet.exact_peers.clone();
    resolve_magnet_from_peers(add, magnet, peers, peer_id, config).await
}

pub(crate) async fn resolve_magnet_from_peers(
    add: MagnetAdd,
    magnet: MagnetUri,
    peers: Vec<std::net::SocketAddr>,
    peer_id: PeerId,
    config: MetadataFetchConfig,
) -> Result<ResolvedMagnet, RuntimeError> {
    let info_hash = magnet.info_hash_v1.ok_or_else(|| {
        RuntimeError::Magnet("v1 info hash is required for metadata fetch".into())
    })?;
    if peers.is_empty() {
        return Err(RuntimeError::Magnet(
            "magnet metadata resolution requires at least one peer".into(),
        ));
    }

    let mut last_error = None;
    for peer in peers {
        match fetch_metadata_from_peer(peer, info_hash, peer_id, config).await {
            Ok(bytes) => {
                let metainfo = decode_torrent(&bytes)?;
                validate_magnet_info_hash(info_hash, metainfo.info_hash_v1)?;
                let plan = TorrentPlan::from_metainfo_decentralized(metainfo, &add.destination)?;
                return Ok(ResolvedMagnet { magnet, plan });
            }
            Err(err) => last_error = Some(err.to_string()),
        }
    }

    Err(RuntimeError::Magnet(format!(
        "all exact peers failed metadata resolution: {}",
        last_error.unwrap_or_else(|| "no peers attempted".to_owned())
    )))
}

fn validate_magnet_info_hash(expected: InfoHashV1, actual: InfoHashV1) -> Result<(), RuntimeError> {
    if expected != actual {
        return Err(RuntimeError::Magnet(
            "resolved metadata info hash did not match magnet xt".into(),
        ));
    }
    Ok(())
}

impl ResolvedMagnet {
    #[must_use]
    pub fn id(&self) -> TorrentId {
        self.plan.id
    }
}
