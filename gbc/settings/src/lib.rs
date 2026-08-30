pub mod field;

use nerust_settings_traits::SystemSettings;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GbcCoreSettings {
    pub hardware_model: HardwareModel,
    pub rtc_sync: RtcSyncMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GbcSettings {
    pub core: GbcCoreSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareModel {
    Dmg0,
    Dmg,
    CgbC,
    #[default]
    CgbD,
    Agb,
}

impl std::fmt::Display for HardwareModel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Dmg0 => "dmg0",
            Self::Dmg => "dmg",
            Self::CgbC => "cgb_c",
            Self::CgbD => "cgb_d",
            Self::Agb => "agb",
        })
    }
}

impl std::str::FromStr for HardwareModel {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "dmg0" => Ok(Self::Dmg0),
            "dmg" => Ok(Self::Dmg),
            "cgb-c" | "cgb_c" => Ok(Self::CgbC),
            "cgb-d" | "cgb_d" => Ok(Self::CgbD),
            "agb" => Ok(Self::Agb),
            _ => Err(format!("unknown GBC hardware model: {value}")),
        }
    }
}

/// RTC (Real-Time Clock) synchronization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RtcSyncMode {
    Off,
    SaveDataOnly,
    #[default]
    SystemTime,
}

impl GbcSettings {
    pub fn set_rtc_sync(&mut self, v: RtcSyncMode) {
        self.core.rtc_sync = v;
    }
}

#[typetag::serde]
impl SystemSettings for GbcSettings {
    fn requires_live_session_rebuild(&self, next: &dyn SystemSettings) -> bool {
        next.downcast_ref::<GbcSettings>()
            .is_some_and(|other| self.core.hardware_model != other.core.hardware_model)
    }
}

#[cfg(test)]
mod tests {
    use nerust_settings_traits::SystemSettings;

    use super::*;

    fn test_settings() -> GbcSettings {
        GbcSettings {
            core: GbcCoreSettings {
                hardware_model: HardwareModel::Dmg,
                rtc_sync: RtcSyncMode::SystemTime,
            },
        }
    }

    #[test]
    fn default_syncs_rtc_with_system_time() {
        let s = GbcSettings::default();
        assert_eq!(s.core.hardware_model, HardwareModel::CgbD);
        assert_eq!(s.core.rtc_sync, RtcSyncMode::SystemTime);
    }

    #[test]
    fn dyn_clone_preserves_values() {
        let settings: Box<dyn SystemSettings> = Box::new(test_settings());
        let cloned = settings.clone();
        let cloned_gbc = cloned
            .downcast_ref::<GbcSettings>()
            .expect("cloned should downcast");

        assert_eq!(cloned_gbc, &test_settings());
    }

    #[test]
    fn set_rtc_sync_updates_field() {
        let mut s = GbcSettings::default();
        s.set_rtc_sync(RtcSyncMode::SystemTime);
        assert_eq!(s.core.rtc_sync, RtcSyncMode::SystemTime);
    }

    #[test]
    fn requires_live_session_rebuild_ignores_rtc_change() {
        let a: GbcSettings = test_settings();
        let mut b = a.clone();
        b.core.rtc_sync = RtcSyncMode::Off;

        assert!(!a.requires_live_session_rebuild(&b));
    }

    #[test]
    fn requires_live_session_rebuild_detects_hardware_model_change() {
        let a = test_settings();
        let mut b = a.clone();
        b.core.hardware_model = HardwareModel::Agb;

        assert!(a.requires_live_session_rebuild(&b));
    }

    #[test]
    fn hardware_model_parses_cli_and_choice_ids() {
        assert_eq!("cgb-d".parse(), Ok(HardwareModel::CgbD));
        assert_eq!("cgb_d".parse(), Ok(HardwareModel::CgbD));
        assert!("unknown".parse::<HardwareModel>().is_err());
    }

    #[test]
    fn typetag_serialization_does_not_conflict_with_fields() {
        let settings: Box<dyn SystemSettings> = Box::new(test_settings());
        assert!(serde_value::to_value(&settings).is_ok());
    }
}
