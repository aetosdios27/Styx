use crate::{RuntimeLimits, SeedPolicy};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsPatch {
    pub listen_port: Option<u16>,
    pub limits: Option<RuntimeLimits>,
    pub seed_policy: Option<SeedPolicy>,
}
