use nerust_core_traits::CoreOptions;
use nerust_gbc_settings::HardwareModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RtcSyncPolicy {
    Off,
    SaveDataOnly,
    SystemTime,
}

impl RtcSyncPolicy {
    pub fn syncs_save_data(self) -> bool {
        matches!(self, Self::SaveDataOnly | Self::SystemTime)
    }

    pub fn syncs_snapshots(self) -> bool {
        self == Self::SystemTime
    }
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
            rtc_sync: RtcSyncPolicy::SystemTime,
        }
    }
}

impl CoreOptions for GbcCoreOptions {}

#[cfg(test)]
mod tests {
    use super::RtcSyncPolicy;

    #[test]
    fn rtc_sync_policies_cover_save_data_and_snapshots() {
        assert!(!RtcSyncPolicy::Off.syncs_save_data());
        assert!(!RtcSyncPolicy::Off.syncs_snapshots());
        assert!(RtcSyncPolicy::SaveDataOnly.syncs_save_data());
        assert!(!RtcSyncPolicy::SaveDataOnly.syncs_snapshots());
        assert!(RtcSyncPolicy::SystemTime.syncs_save_data());
        assert!(RtcSyncPolicy::SystemTime.syncs_snapshots());
    }
}
