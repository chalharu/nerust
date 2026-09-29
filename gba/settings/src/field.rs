use std::sync::Arc;

use nerust_core_traits::factory::descriptor::{
    SystemSettingsChoiceId, SystemSettingsChoiceOption, SystemSettingsFieldId,
};
use strum::{Display, EnumIter, EnumString};

use crate::{GbaSettings, SolarLight};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, Display, EnumIter)]
pub enum GbaSettingField {
    #[strum(serialize = "peripheral.solar_light")]
    PeripheralSolarLight,
}

impl GbaSettingField {
    pub fn label_id(&self) -> &'static str {
        match self {
            Self::PeripheralSolarLight => "gba.peripheral.solar_light",
        }
    }

    pub fn field_id(&self) -> SystemSettingsFieldId {
        SystemSettingsFieldId(std::borrow::Cow::Owned(self.to_string()))
    }

    pub fn current_choice(&self, settings: &GbaSettings) -> SystemSettingsChoiceId {
        let id = match self {
            Self::PeripheralSolarLight => {
                GbaSettingChoice::from(settings.core.solar_light).to_string()
            }
        };
        SystemSettingsChoiceId(std::borrow::Cow::Owned(id))
    }

    pub fn options(&self) -> Arc<[SystemSettingsChoiceOption]> {
        use GbaSettingChoice::*;
        let list = match self {
            Self::PeripheralSolarLight => [Blinding, Bright, Normal, Dim, Dark],
        };
        Arc::from(
            list.iter()
                .map(|c| SystemSettingsChoiceOption {
                    id: SystemSettingsChoiceId(std::borrow::Cow::Owned(c.to_string())),
                    label_id: c.label_id(),
                })
                .collect::<Vec<_>>(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, Display, EnumIter)]
pub enum GbaSettingChoice {
    #[strum(serialize = "blinding")]
    Blinding,
    #[strum(serialize = "bright")]
    Bright,
    #[strum(serialize = "normal")]
    Normal,
    #[strum(serialize = "dim")]
    Dim,
    #[strum(serialize = "dark")]
    Dark,
}

impl GbaSettingChoice {
    pub fn label_id(&self) -> &'static str {
        match self {
            Self::Blinding => "gba.solar.blinding",
            Self::Bright => "gba.solar.bright",
            Self::Normal => "gba.solar.normal",
            Self::Dim => "gba.solar.dim",
            Self::Dark => "gba.solar.dark",
        }
    }
}

impl From<SolarLight> for GbaSettingChoice {
    fn from(value: SolarLight) -> Self {
        match value {
            SolarLight::Blinding => Self::Blinding,
            SolarLight::Bright => Self::Bright,
            SolarLight::Normal => Self::Normal,
            SolarLight::Dim => Self::Dim,
            SolarLight::Dark => Self::Dark,
        }
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn field_ids_are_unique() {
        let ids: Vec<String> = GbaSettingField::iter().map(|f| f.to_string()).collect();
        let mut dedup = ids.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len());
    }

    #[test]
    fn choice_ids_are_unique() {
        let ids: Vec<String> = GbaSettingChoice::iter().map(|c| c.to_string()).collect();
        let mut dedup = ids.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len());
    }

    #[test]
    fn field_label_ids_are_unique() {
        let labels: Vec<&str> = GbaSettingField::iter().map(|f| f.label_id()).collect();
        let mut dedup = labels.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(labels.len(), dedup.len());
    }

    #[test]
    fn options_returns_non_empty_for_each_field() {
        for field in GbaSettingField::iter() {
            let opts = field.options();
            assert!(!opts.is_empty(), "field {field} has no options");
        }
    }
}
