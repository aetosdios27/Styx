use crate::RuntimeLimits;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsPatch {
    pub listen_port: Option<u16>,
    pub limits: Option<RuntimeLimits>,
}
