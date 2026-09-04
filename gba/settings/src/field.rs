use std::sync::Arc;

use nerust_core_traits::factory::descriptor::{
    SystemSettingsChoiceId, SystemSettingsChoiceOption, SystemSettingsFieldId,
};
use strum::{Display, EnumIter, EnumString};

use crate::GbaSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, Display, EnumIter)]
pub enum GbaSettingField {}

impl GbaSettingField {
    pub fn label_id(&self) -> &'static str {
        match *self {}
    }

    pub fn field_id(&self) -> SystemSettingsFieldId {
        SystemSettingsFieldId(std::borrow::Cow::Owned(self.to_string()))
    }

    pub fn current_choice(&self, _settings: &GbaSettings) -> SystemSettingsChoiceId {
        match *self {}
    }

    pub fn options(&self) -> Arc<[SystemSettingsChoiceOption]> {
        match *self {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, Display, EnumIter)]
pub enum GbaSettingChoice {}

impl GbaSettingChoice {
    pub fn label_id(&self) -> &'static str {
        match *self {}
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
