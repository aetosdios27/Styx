use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use styx_dht::{DhtConfig, DhtError};

#[test]
fn config_defaults_use_bep_shaped_runtime_values() {
    let config = DhtConfig::default();

    assert_eq!(config.query_timeout, Duration::from_secs(15));
    assert_eq!(config.bucket_refresh_interval, Duration::from_secs(15 * 60));
    assert_eq!(config.lookup_alpha, 3);
}

#[test]
fn config_validation_rejects_zero_transaction_capacity() {
    let mut config = DhtConfig::default();
    config.max_transactions = 0;

    assert_eq!(
        config.validate().unwrap_err(),
        DhtError::InvalidConfig("max_transactions")
    );
}

#[test]
fn config_keeps_bootstrap_nodes_explicit_and_deduplicated() {
    let mut config = DhtConfig::default();
    let bootstrap = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)), 6881);

    config.add_bootstrap_node(bootstrap);
    config.add_bootstrap_node(bootstrap);

    assert_eq!(config.bootstrap_nodes(), &[bootstrap]);
}
