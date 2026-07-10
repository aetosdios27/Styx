use styx_proto::TorrentMetainfo;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryPolicy {
    decentralized: bool,
}

impl DiscoveryPolicy {
    #[must_use]
    pub const fn from_metainfo(metainfo: &TorrentMetainfo) -> Self {
        Self {
            decentralized: !metainfo.info.private,
        }
    }

    #[must_use]
    pub const fn dht_allowed(self) -> bool {
        self.decentralized
    }

    #[must_use]
    pub const fn pex_allowed(self) -> bool {
        self.decentralized
    }

    #[must_use]
    pub const fn lsd_allowed(self) -> bool {
        self.decentralized
    }

    #[must_use]
    pub const fn port_message_allowed(self) -> bool {
        self.decentralized
    }
}
