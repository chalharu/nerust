use std::sync::Arc;

use nerust_core_traits::factory::descriptor::{
    SystemSettingsChoiceId, SystemSettingsChoiceOption, SystemSettingsFieldId,
};
use strum::{Display, EnumIter, EnumString};

use crate::{GbcSettings, HardwareModel, RtcSyncMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, Display, EnumIter)]
pub enum GbcSettingField {
    #[strum(serialize = "system.hardware_model")]
    SystemHardwareModel,
    #[strum(serialize = "system.rtc_sync")]
    SystemRtcSync,
}

impl GbcSettingField {
    pub fn current_choice(&self, s: &GbcSettings) -> SystemSettingsChoiceId {
        let id = match self {
            Self::SystemHardwareModel => {
                GbcSettingChoice::from(s.system.hardware_model).to_string()
            }
            Self::SystemRtcSync => GbcSettingChoice::from(s.system.rtc_sync).to_string(),
        };
        SystemSettingsChoiceId(std::borrow::Cow::Owned(id))
    }

    pub fn options(&self) -> Arc<[SystemSettingsChoiceOption]> {
        let list = match self {
            Self::SystemHardwareModel => hardware_model_options(),
            Self::SystemRtcSync => rtc_sync_options(),
        };
        Arc::from(list)
    }

    pub fn label_id(&self) -> &'static str {
        match self {
            Self::SystemHardwareModel => "gbc.system.hardware_model",
            Self::SystemRtcSync => "gbc.system.rtc_sync",
        }
    }

    pub fn field_id(&self) -> SystemSettingsFieldId {
        SystemSettingsFieldId(std::borrow::Cow::Owned(self.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, Display)]
pub enum GbcSettingChoice {
    #[strum(serialize = "dmg0")]
    Dmg0,
    #[strum(serialize = "dmg")]
    Dmg,
    #[strum(serialize = "cgb_c")]
    CgbC,
    #[strum(serialize = "cgb_d")]
    CgbD,
    #[strum(serialize = "agb")]
    Agb,
    #[strum(serialize = "off")]
    Off,
    #[strum(serialize = "system_time")]
    SystemTime,
}

impl GbcSettingChoice {
    pub fn label_id(&self) -> &'static str {
        match self {
            Self::Dmg0 => "gbc.hardware.dmg0",
            Self::Dmg => "gbc.hardware.dmg",
            Self::CgbC => "gbc.hardware.cgb_c",
            Self::CgbD => "gbc.hardware.cgb_d",
            Self::Agb => "gbc.hardware.agb",
            Self::Off => "gbc.rtc_sync.off",
            Self::SystemTime => "gbc.rtc_sync.system_time",
        }
    }
}

impl From<HardwareModel> for GbcSettingChoice {
    fn from(value: HardwareModel) -> Self {
        match value {
            HardwareModel::Dmg0 => Self::Dmg0,
            HardwareModel::Dmg => Self::Dmg,
            HardwareModel::CgbC => Self::CgbC,
            HardwareModel::CgbD => Self::CgbD,
            HardwareModel::Agb => Self::Agb,
        }
    }
}

impl From<RtcSyncMode> for GbcSettingChoice {
    fn from(v: RtcSyncMode) -> Self {
        match v {
            RtcSyncMode::Off => Self::Off,
            RtcSyncMode::SystemTime => Self::SystemTime,
        }
    }
}

fn build_choice_options(choices: &[GbcSettingChoice]) -> Vec<SystemSettingsChoiceOption> {
    choices
        .iter()
        .map(|c| SystemSettingsChoiceOption {
            id: SystemSettingsChoiceId(std::borrow::Cow::Owned(c.to_string())),
            label_id: c.label_id(),
        })
        .collect()
}

fn hardware_model_options() -> Vec<SystemSettingsChoiceOption> {
    use GbcSettingChoice::*;
    build_choice_options(&[Dmg0, Dmg, CgbC, CgbD, Agb])
}

fn rtc_sync_options() -> Vec<SystemSettingsChoiceOption> {
    use GbcSettingChoice::*;
    build_choice_options(&[Off, SystemTime])
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn field_ids_are_unique() {
        let ids: Vec<String> = GbcSettingField::iter().map(|f| f.to_string()).collect();
        let mut dedup = ids.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len());
    }

    #[test]
    fn choice_ids_are_unique() {
        let all = [
            GbcSettingChoice::Dmg0,
            GbcSettingChoice::Dmg,
            GbcSettingChoice::CgbC,
            GbcSettingChoice::CgbD,
            GbcSettingChoice::Agb,
            GbcSettingChoice::Off,
            GbcSettingChoice::SystemTime,
        ];
        let ids: Vec<String> = all.iter().map(|c| c.to_string()).collect();
        let mut dedup = ids.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len());
    }

    #[test]
    fn field_label_ids_are_unique() {
        let labels: Vec<&str> = GbcSettingField::iter().map(|f| f.label_id()).collect();
        let mut dedup = labels.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(labels.len(), dedup.len());
    }

    #[test]
    fn options_returns_non_empty_for_each_field() {
        for field in GbcSettingField::iter() {
            let opts = field.options();
            assert!(!opts.is_empty(), "field {field} has no options");
        }
    }

    #[test]
    fn current_choice_matches_rtc_system_time() {
        let mut s = GbcSettings::default();
        s.set_rtc_sync(RtcSyncMode::SystemTime);
        let c = GbcSettingField::SystemRtcSync.current_choice(&s);
        assert_eq!(c.as_str(), "system_time");
    }

    #[test]
    fn current_choice_matches_hardware_model() {
        let mut settings = GbcSettings::default();
        settings.system.hardware_model = HardwareModel::Agb;
        let choice = GbcSettingField::SystemHardwareModel.current_choice(&settings);
        assert_eq!(choice.as_str(), "agb");
    }

    #[test]
    fn from_rtc_sync_mode_maps_all_variants() {
        assert_eq!(GbcSettingChoice::from(RtcSyncMode::Off).to_string(), "off");
        assert_eq!(
            GbcSettingChoice::from(RtcSyncMode::SystemTime).to_string(),
            "system_time"
        );
    }
}
