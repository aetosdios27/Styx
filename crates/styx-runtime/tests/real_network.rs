use std::{env, path::PathBuf};

use styx_runtime::{run_full_v1_download, run_one_piece_smoke, DownloadRunConfig, SmokeRunConfig};

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

fn required_path(name: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must point to a legal v1 torrent smoke-test fixture"))
}
