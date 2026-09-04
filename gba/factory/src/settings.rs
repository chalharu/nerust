use crate::GbaLoadOptions;
use nerust_core_traits::factory::{
    FactoryError,
    descriptor::{
        SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsFieldKind,
        SystemSettingsFieldModel, SystemSettingsPageModel,
    },
    load::{DynSystemLoadOptions, DynSystemLoadOptionsExt, ResolvedLoadRequest},
    settings::FactorySettingsView,
};
use nerust_gba_core::core_options::GbaCoreOptions;
use nerust_gba_settings::{
    GbaSettings,
    field::{GbaSettingChoice, GbaSettingField},
};

const EXPOSED_FIELDS: [GbaSettingField; 0] = [];

pub(crate) fn gba_settings_page(view: &FactorySettingsView) -> SystemSettingsPageModel {
    let defaults = GbaSettings;
    let settings = view
        .system_config
        .as_deref()
        .and_then(|value| value.downcast_ref::<GbaSettings>())
        .unwrap_or(&defaults);
    SystemSettingsPageModel {
        fields: EXPOSED_FIELDS
            .iter()
            .map(|field| SystemSettingsFieldModel {
                id: field.field_id(),
                label_id: field.label_id(),
                kind: SystemSettingsFieldKind::Choice {
                    selected: field.current_choice(settings),
                    options: field.options(),
                },
            })
            .collect::<Vec<_>>()
            .into(),
    }
}

pub(crate) fn apply_gba_settings_choice(
    view: &mut FactorySettingsView,
    field_id: &SystemSettingsFieldId,
    choice_id: &SystemSettingsChoiceId,
) -> Result<(), FactoryError> {
    let field = field_id
        .as_str()
        .parse::<GbaSettingField>()
        .map_err(|_| FactoryError::InvalidChoice(field_id.as_str().to_string()))?;
    if !EXPOSED_FIELDS.contains(&field) {
        return Err(FactoryError::InvalidChoice(field_id.as_str().to_string()));
    }
    let _choice = choice_id
        .as_str()
        .parse::<GbaSettingChoice>()
        .map_err(|_| FactoryError::InvalidChoice(choice_id.as_str().to_string()))?;
    let _settings = view
        .system_config
        .as_deref_mut()
        .and_then(|value| value.downcast_mut::<GbaSettings>())
        .ok_or(FactoryError::InvalidSettings)?;
    // TODO(gba-settings): EXPOSED_FIELDS is currently empty (Phase 2).
    // When adding a new field (e.g. GbaVideoFilter), extend the match below
    // to handle (field, choice) pairs, mirroring gbc/factory/src/settings.rs:66-92.
    // At that time, ensure labels.rs and field.rs label_id mappings are updated together.
    Ok(())
}

pub(crate) fn resolve_gba_load_request(
    view: &FactorySettingsView,
    options: Box<dyn DynSystemLoadOptions>,
) -> Result<ResolvedLoadRequest, FactoryError> {
    let _settings = view
        .system_config
        .as_deref()
        .and_then(|value| value.downcast_ref::<GbaSettings>())
        .ok_or(FactoryError::InvalidSettings)?;
    let _options = options
        .into_inner::<GbaLoadOptions>()
        .map_err(|_| FactoryError::Resolve("failed to downcast GBA load options".to_string()))?;
    Ok(ResolvedLoadRequest {
        options: GbaCoreOptions.into(),
    })
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use nerust_core_traits::factory::settings::Language;

    use super::*;

    fn view() -> FactorySettingsView {
        FactorySettingsView {
            language: Language::SystemDefault,
            system_config: Some(Box::new(GbaSettings)),
        }
    }

    #[test]
    fn page_exposes_no_fields() {
        let page = gba_settings_page(&view());
        assert_eq!(page.fields.len(), 0);
    }

    #[test]
    fn apply_rejects_unknown_field() {
        let mut v = view();
        assert!(
            apply_gba_settings_choice(
                &mut v,
                &SystemSettingsFieldId(Cow::Borrowed("system.unknown")),
                &SystemSettingsChoiceId(Cow::Borrowed("any")),
            )
            .is_err()
        );
    }

    #[test]
    fn resolves_with_default_options() {
        let view = view();
        let resolved = resolve_gba_load_request(&view, GbaLoadOptions.into()).unwrap();
        assert!(resolved.options.downcast_ref::<GbaCoreOptions>().is_some());
    }
}
