use std::sync::Arc;

use nerust_core_traits::factory::descriptor::{
    SystemSettingsChoiceId, SystemSettingsChoiceOption, SystemSettingsFieldId,
};
use strum::{Display, EnumIter, EnumString};

use crate::{DmgPalette, GbcSettings, RtcSyncMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, Display, EnumIter)]
pub enum GbcSettingField {
    #[strum(serialize = "video.dmg_palette")]
    VideoDmgPalette,
    #[strum(serialize = "video.interframe_blending")]
    VideoInterframeBlending,
    #[strum(serialize = "system.boot_rom_enabled")]
    SystemBootRomEnabled,
    #[strum(serialize = "system.rtc_sync")]
    SystemRtcSync,
}

impl GbcSettingField {
    pub fn current_choice(&self, s: &GbcSettings) -> SystemSettingsChoiceId {
        let id = match self {
            Self::VideoDmgPalette => GbcSettingChoice::from(s.video.dmg_palette).to_string(),
            Self::VideoInterframeBlending => {
                GbcSettingChoice::from_bool(s.video.interframe_blending).to_string()
            }
            Self::SystemBootRomEnabled => {
                GbcSettingChoice::from_bool(s.system.boot_rom_enabled).to_string()
            }
            Self::SystemRtcSync => GbcSettingChoice::from(s.system.rtc_sync).to_string(),
        };
        SystemSettingsChoiceId(std::borrow::Cow::Owned(id))
    }

    pub fn options(&self) -> Arc<[SystemSettingsChoiceOption]> {
        let list = match self {
            Self::VideoDmgPalette => dmg_palette_options(),
            Self::VideoInterframeBlending => bool_options(),
            Self::SystemBootRomEnabled => bool_options(),
            Self::SystemRtcSync => rtc_sync_options(),
        };
        Arc::from(list)
    }

    pub fn label_id(&self) -> &'static str {
        match self {
            Self::VideoDmgPalette => "gbc.video.dmg_palette",
            Self::VideoInterframeBlending => "gbc.video.interframe_blending",
            Self::SystemBootRomEnabled => "gbc.system.boot_rom_enabled",
            Self::SystemRtcSync => "gbc.system.rtc_sync",
        }
    }

    pub fn field_id(&self) -> SystemSettingsFieldId {
        SystemSettingsFieldId(std::borrow::Cow::Owned(self.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, Display)]
pub enum GbcSettingChoice {
    #[strum(serialize = "greyscale")]
    Greyscale,
    #[strum(serialize = "green_tint")]
    GreenTint,
    #[strum(serialize = "brown_tint")]
    BrownTint,
    #[strum(serialize = "pastel_mix")]
    PastelMix,
    #[strum(serialize = "inverted")]
    Inverted,
    #[strum(serialize = "on")]
    On,
    #[strum(serialize = "off")]
    Off,
    #[strum(serialize = "system_time")]
    SystemTime,
}

impl GbcSettingChoice {
    pub fn label_id(&self) -> &'static str {
        match self {
            Self::Greyscale => "gbc.palette.greyscale",
            Self::GreenTint => "gbc.palette.green_tint",
            Self::BrownTint => "gbc.palette.brown_tint",
            Self::PastelMix => "gbc.palette.pastel_mix",
            Self::Inverted => "gbc.palette.inverted",
            Self::On => "gbc.boolean.on",
            Self::Off => "gbc.boolean.off",
            Self::SystemTime => "gbc.rtc_sync.system_time",
        }
    }

    fn from_bool(v: bool) -> Self {
        if v { Self::On } else { Self::Off }
    }
}

impl From<DmgPalette> for GbcSettingChoice {
    fn from(v: DmgPalette) -> Self {
        match v {
            DmgPalette::Greyscale => Self::Greyscale,
            DmgPalette::GreenTint => Self::GreenTint,
            DmgPalette::BrownTint => Self::BrownTint,
            DmgPalette::PastelMix => Self::PastelMix,
            DmgPalette::Inverted => Self::Inverted,
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

fn dmg_palette_options() -> Vec<SystemSettingsChoiceOption> {
    use GbcSettingChoice::*;
    build_choice_options(&[Greyscale, GreenTint, BrownTint, PastelMix, Inverted])
}

fn bool_options() -> Vec<SystemSettingsChoiceOption> {
    build_choice_options(&[GbcSettingChoice::On, GbcSettingChoice::Off])
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
            GbcSettingChoice::Greyscale,
            GbcSettingChoice::GreenTint,
            GbcSettingChoice::BrownTint,
            GbcSettingChoice::PastelMix,
            GbcSettingChoice::Inverted,
            GbcSettingChoice::On,
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
    fn current_choice_matches_palette_setting() {
        let s = GbcSettings::default();
        let c = GbcSettingField::VideoDmgPalette.current_choice(&s);
        assert_eq!(c.as_str(), "greyscale");
    }

    #[test]
    fn current_choice_matches_blending_setting() {
        let mut s = GbcSettings::default();
        s.set_interframe_blending(true);
        let c = GbcSettingField::VideoInterframeBlending.current_choice(&s);
        assert_eq!(c.as_str(), "on");
    }

    #[test]
    fn current_choice_matches_boot_rom_off_by_default() {
        let s = GbcSettings::default();
        let c = GbcSettingField::SystemBootRomEnabled.current_choice(&s);
        assert_eq!(c.as_str(), "off");
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
    fn from_bool_true_is_on() {
        assert_eq!(GbcSettingChoice::from_bool(true).to_string(), "on");
    }

    #[test]
    fn from_bool_false_is_off() {
        assert_eq!(GbcSettingChoice::from_bool(false).to_string(), "off");
    }

    #[test]
    fn from_dmg_palette_maps_all_variants() {
        let pairs = [
            (DmgPalette::Greyscale, "greyscale"),
            (DmgPalette::GreenTint, "green_tint"),
            (DmgPalette::BrownTint, "brown_tint"),
            (DmgPalette::PastelMix, "pastel_mix"),
            (DmgPalette::Inverted, "inverted"),
        ];
        for (palette, expected) in pairs {
            assert_eq!(GbcSettingChoice::from(palette).to_string(), expected);
        }
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
