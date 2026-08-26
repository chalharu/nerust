use nerust_core_traits::CoreOptions;

use crate::system::HardwareModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RtcSyncPolicy {
    Off,
    SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GbcCoreOptions {
    pub hardware_model: HardwareModel,
    pub rtc_sync: RtcSyncPolicy,
}

impl Default for GbcCoreOptions {
    fn default() -> Self {
        Self {
            hardware_model: HardwareModel::CgbD,
            rtc_sync: RtcSyncPolicy::Off,
        }
    }
}

impl CoreOptions for GbcCoreOptions {}
