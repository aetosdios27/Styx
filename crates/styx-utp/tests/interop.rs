#[test]
#[ignore = "requires an externally configured libutp/libtorrent-compatible endpoint"]
fn reference_utp_interop_harness_placeholder() {
    let Ok(endpoint) = std::env::var("STYX_UTP_INTEROP_ENDPOINT") else {
        return;
    };

    assert!(!endpoint.trim().is_empty());
}
