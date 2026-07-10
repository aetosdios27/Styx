use std::{env, path::PathBuf};

use std::time::{Duration, Instant};

use styx_app::{ControlCommand, TorrentRuntime};
use styx_runtime::{
    run_full_v1_download, run_one_piece_smoke, spawn_dht_worker, AppRuntime, DhtRuntimeConfig,
    DownloadRunConfig, RuntimeConfig, SmokeRunConfig,
};

#[tokio::test]
#[ignore = "requires STYX_REAL_TORRENT and STYX_REAL_DEST pointing to a legal v1 torrent fixture"]
async fn real_network_smoke_downloads_and_verifies_one_piece() {
    let torrent_path = required_path("STYX_REAL_TORRENT");
    let destination = required_path("STYX_REAL_DEST");
    let listen_port = env::var("STYX_REAL_LISTEN_PORT")
        .ok()
        .map(|raw| {
            raw.parse::<u16>()
                .expect("STYX_REAL_LISTEN_PORT must be a u16")
        })
        .unwrap_or(6881);

    let mut config = SmokeRunConfig::default_for_paths(torrent_path, destination);
    config.listen_port = listen_port;

    let outcome = run_one_piece_smoke(config).await.unwrap();

    assert_eq!(outcome.piece(), 0);
}

#[tokio::test]
#[ignore = "requires STYX_REAL_FULL=1 plus legal small v1 torrent fixture env vars"]
async fn real_network_full_download_completes_small_legal_v1_torrent() {
    assert_eq!(
        env::var("STYX_REAL_FULL").as_deref(),
        Ok("1"),
        "set STYX_REAL_FULL=1 to acknowledge this performs a full legal torrent download"
    );
    let torrent_path = required_path("STYX_REAL_TORRENT");
    let destination = required_path("STYX_REAL_DEST");
    let listen_port = env::var("STYX_REAL_LISTEN_PORT")
        .ok()
        .map(|raw| {
            raw.parse::<u16>()
                .expect("STYX_REAL_LISTEN_PORT must be a u16")
        })
        .unwrap_or(6881);

    let mut config = DownloadRunConfig::default_for_paths(torrent_path, destination);
    config.listen_port = listen_port;

    let outcome = run_full_v1_download(config).await.unwrap();

    assert!(outcome.pieces() > 0);
    assert!(outcome.bytes() > 0);
}

#[tokio::test]
#[ignore = "requires STYX_REAL_DHT=1, STYX_REAL_MAGNET, and STYX_REAL_DEST for legal public data"]
async fn real_network_magnet_resolves_metadata_and_downloads_one_piece() {
    assert_eq!(
        env::var("STYX_REAL_DHT").as_deref(),
        Ok("1"),
        "set STYX_REAL_DHT=1 to acknowledge public DHT traffic"
    );
    let magnet = env::var("STYX_REAL_MAGNET").expect("STYX_REAL_MAGNET must be a legal magnet");
    let destination = required_path("STYX_REAL_DEST");
    let bootstrap = tokio::net::lookup_host("router.bittorrent.com:6881")
        .await
        .expect("public DHT bootstrap DNS failed")
        .next()
        .expect("public DHT bootstrap returned no addresses");
    let dht = DhtRuntimeConfig {
        bootstrap_nodes: vec![bootstrap],
        ..DhtRuntimeConfig::default()
    };
    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
    let worker = spawn_dht_worker(dht.clone(), events_tx).await.unwrap();
    let mut runtime = AppRuntime::new_with_config(RuntimeConfig {
        dht,
        source_timeout: Duration::from_secs(30),
        ..RuntimeConfig::default()
    })
    .unwrap();
    runtime
        .attach_dht_worker(worker.clone(), events_rx)
        .unwrap();
    runtime
        .apply(ControlCommand::AddMagnet {
            uri: magnet,
            destination: Some(destination),
        })
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(120);
    let mut downloaded = false;
    while Instant::now() < deadline {
        runtime.tick();
        downloaded = runtime
            .snapshot()
            .torrents
            .first()
            .is_some_and(|torrent| torrent.progress > 0.0);
        if downloaded {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    worker.shutdown().await.unwrap();
    assert!(
        downloaded,
        "magnet made no verified progress before timeout"
    );
}

fn required_path(name: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must point to a legal v1 torrent smoke-test fixture"))
}
