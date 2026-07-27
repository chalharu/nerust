use nerust_settings_traits::SystemSettings;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GbcVideoSettings {
    pub dmg_palette: DmgPalette,
    pub interframe_blending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GbcSystemSettingsSection {
    pub boot_rom_enabled: bool,
    pub rtc_sync: RtcSyncMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GbcSettings {
    pub video: GbcVideoSettings,
    pub system: GbcSystemSettingsSection,
}

/// DMG (original Game Boy) color palette presets.
///
/// Used when running a monochrome Game Boy ROM in GBC mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmgPalette {
    #[default]
    Greyscale,
    GreenTint,
    BrownTint,
    PastelMix,
    Inverted,
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
    pub fn set_dmg_palette(&mut self, v: DmgPalette) {
        self.video.dmg_palette = v;
    }

    pub fn set_interframe_blending(&mut self, v: bool) {
        self.video.interframe_blending = v;
    }

    pub fn set_boot_rom_enabled(&mut self, v: bool) {
        self.system.boot_rom_enabled = v;
    }

    pub fn set_rtc_sync(&mut self, v: RtcSyncMode) {
        self.system.rtc_sync = v;
    }
}

#[typetag::serde]
impl SystemSettings for GbcSettings {
    fn requires_live_session_rebuild(&self, next: &dyn SystemSettings) -> bool {
        if let Some(other) = next.downcast_ref::<GbcSettings>() {
            self.video.dmg_palette != other.video.dmg_palette
                || self.video.interframe_blending != other.video.interframe_blending
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use nerust_settings_traits::SystemSettings;

    use super::*;

    fn test_settings() -> GbcSettings {
        GbcSettings {
            video: GbcVideoSettings {
                dmg_palette: DmgPalette::PastelMix,
                interframe_blending: true,
            },
            system: GbcSystemSettingsSection {
                boot_rom_enabled: true,
                rtc_sync: RtcSyncMode::SystemTime,
            },
        }
    }

    #[test]
    fn default_is_greyscale_no_blending_no_bootrom_no_rtc() {
        let s = GbcSettings::default();
        assert_eq!(s.video.dmg_palette, DmgPalette::Greyscale);
        assert!(!s.video.interframe_blending);
        assert!(!s.system.boot_rom_enabled);
        assert_eq!(s.system.rtc_sync, RtcSyncMode::Off);
    }

    #[test]
    fn dyn_clone_preserves_values() {
        let settings: Box<dyn SystemSettings> = Box::new(test_settings());
        let cloned = settings.clone();
        let cloned_gbc = cloned
            .downcast_ref::<GbcSettings>()
            .expect("cloned should downcast");

        assert_eq!(cloned_gbc.video.dmg_palette, DmgPalette::PastelMix);
        assert!(cloned_gbc.video.interframe_blending);
        assert!(cloned_gbc.system.boot_rom_enabled);
    }

    #[test]
    fn set_dmg_palette_updates_field() {
        let mut s = GbcSettings::default();
        s.set_dmg_palette(DmgPalette::Inverted);
        assert_eq!(s.video.dmg_palette, DmgPalette::Inverted);
    }

    #[test]
    fn set_interframe_blending_updates_field() {
        let mut s = GbcSettings::default();
        s.set_interframe_blending(true);
        assert!(s.video.interframe_blending);
    }

    #[test]
    fn set_boot_rom_enabled_updates_field() {
        let mut s = GbcSettings::default();
        s.set_boot_rom_enabled(true);
        assert!(s.system.boot_rom_enabled);
    }

    #[test]
    fn set_rtc_sync_updates_field() {
        let mut s = GbcSettings::default();
        s.set_rtc_sync(RtcSyncMode::SystemTime);
        assert_eq!(s.system.rtc_sync, RtcSyncMode::SystemTime);
    }

    #[test]
    fn requires_live_session_rebuild_detects_palette_change() {
        let a: GbcSettings = test_settings();
        let mut b = a.clone();
        b.video.dmg_palette = DmgPalette::Inverted;

        assert!(a.requires_live_session_rebuild(&b));
    }

    #[test]
    fn requires_live_session_rebuild_detects_blending_change() {
        let a: GbcSettings = test_settings();
        let mut b = a.clone();
        b.video.interframe_blending = false;

        assert!(a.requires_live_session_rebuild(&b));
    }

    #[test]
    fn requires_live_session_rebuild_ignores_bootrom_change() {
        let a: GbcSettings = test_settings();
        let mut b = a.clone();
        b.system.boot_rom_enabled = false;

        assert!(!a.requires_live_session_rebuild(&b));
    }

    #[test]
    fn requires_live_session_rebuild_ignores_rtc_change() {
        let a: GbcSettings = test_settings();
        let mut b = a.clone();
        b.system.rtc_sync = RtcSyncMode::Off;

        assert!(!a.requires_live_session_rebuild(&b));
    }
}
