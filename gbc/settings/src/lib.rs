pub mod field;

use nerust_settings_traits::SystemSettings;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GbcSystemSettingsSection {
    pub rtc_sync: RtcSyncMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GbcSettings {
    pub system: GbcSystemSettingsSection,
}

/// RTC (Real-Time Clock) synchronization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RtcSyncMode {
    #[default]
    Off,
    SystemTime,
}

impl GbcSettings {
    pub fn set_rtc_sync(&mut self, v: RtcSyncMode) {
        self.system.rtc_sync = v;
    }
}

#[typetag::serde]
impl SystemSettings for GbcSettings {
    fn requires_live_session_rebuild(&self, _next: &dyn SystemSettings) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use nerust_settings_traits::SystemSettings;

    use super::*;

    fn test_settings() -> GbcSettings {
        GbcSettings {
            system: GbcSystemSettingsSection {
                rtc_sync: RtcSyncMode::SystemTime,
            },
        }
    }

    #[test]
    fn default_has_rtc_disabled() {
        let s = GbcSettings::default();
        assert_eq!(s.system.rtc_sync, RtcSyncMode::Off);
    }

    #[test]
    fn dyn_clone_preserves_values() {
        let settings: Box<dyn SystemSettings> = Box::new(test_settings());
        let cloned = settings.clone();
        let cloned_gbc = cloned
            .downcast_ref::<GbcSettings>()
            .expect("cloned should downcast");

        assert_eq!(cloned_gbc.system.rtc_sync, RtcSyncMode::SystemTime);
    }

    #[test]
    fn set_rtc_sync_updates_field() {
        let mut s = GbcSettings::default();
        s.set_rtc_sync(RtcSyncMode::SystemTime);
        assert_eq!(s.system.rtc_sync, RtcSyncMode::SystemTime);
    }

    #[test]
    fn requires_live_session_rebuild_ignores_rtc_change() {
        let a: GbcSettings = test_settings();
        let mut b = a.clone();
        b.system.rtc_sync = RtcSyncMode::Off;

        assert!(!a.requires_live_session_rebuild(&b));
    }
}
