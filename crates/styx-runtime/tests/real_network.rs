use std::{env, path::PathBuf};

use styx_runtime::{run_one_piece_smoke, SmokeRunConfig};

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

fn required_path(name: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must point to a legal v1 torrent smoke-test fixture"))
}
