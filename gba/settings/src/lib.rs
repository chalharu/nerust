pub mod field;

use nerust_settings_traits::SystemSettings;
use serde::{Deserialize, Serialize};

/// Boktai solar-sensor light level (GBATEK: 00h = blinding .. E8h = dark).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolarLight {
    Blinding,
    Bright,
    #[default]
    Normal,
    Dim,
    Dark,
}

impl SolarLight {
    pub fn level(self) -> u8 {
        match self {
            Self::Blinding => 0x00,
            Self::Bright => 0x40,
            Self::Normal => 0x60,
            Self::Dim => 0xA0,
            Self::Dark => 0xE0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GbaCoreSettings {
    pub solar_light: SolarLight,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GbaSettings {
    pub core: GbaCoreSettings,
}

#[typetag::serde]
impl SystemSettings for GbaSettings {
    fn requires_live_session_rebuild(&self, _next: &dyn SystemSettings) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use nerust_settings_traits::SystemSettings;

    use super::*;

    #[test]
    fn default_settings() {
        let settings = GbaSettings::default();
        assert_eq!(settings.core.solar_light, SolarLight::Normal);
        assert!(!settings.requires_live_session_rebuild(&GbaSettings::default()));
    }

    #[test]
    fn dyn_clone_preserves_values() {
        let settings: Box<dyn SystemSettings> = Box::new(GbaSettings::default());
        let cloned = settings.clone();
        let cloned_gba = cloned
            .downcast_ref::<GbaSettings>()
            .expect("cloned should downcast");
        assert_eq!(cloned_gba, &GbaSettings::default());
    }

    #[test]
    fn solar_levels_span_blinding_to_dark() {
        assert_eq!(SolarLight::Blinding.level(), 0x00);
        assert_eq!(SolarLight::Dark.level(), 0xE0);
    }
}
