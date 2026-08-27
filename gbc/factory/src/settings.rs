use crate::GbcLoadOptions;
use nerust_core_traits::factory::{
    FactoryError,
    descriptor::{
        SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsFieldKind,
        SystemSettingsFieldModel, SystemSettingsPageModel,
    },
    load::{DynSystemLoadOptions, DynSystemLoadOptionsExt, ResolvedLoadRequest},
    settings::FactorySettingsView,
};
use nerust_gbc_core::core_options::{GbcCoreOptions, RtcSyncPolicy};
use nerust_gbc_settings::{
    GbcSettings, HardwareModel, RtcSyncMode,
    field::{GbcSettingChoice, GbcSettingField},
};

const EXPOSED_FIELDS: [GbcSettingField; 2] = [
    GbcSettingField::SystemHardwareModel,
    GbcSettingField::SystemRtcSync,
];

pub(crate) fn gbc_settings_page(view: &FactorySettingsView) -> SystemSettingsPageModel {
    let defaults = GbcSettings::default();
    let settings = view
        .system_config
        .as_deref()
        .and_then(|value| value.downcast_ref::<GbcSettings>())
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

pub(crate) fn apply_gbc_settings_choice(
    view: &mut FactorySettingsView,
    field_id: &SystemSettingsFieldId,
    choice_id: &SystemSettingsChoiceId,
) -> Result<(), FactoryError> {
    let field = field_id
        .as_str()
        .parse::<GbcSettingField>()
        .map_err(|_| FactoryError::InvalidChoice(field_id.as_str().to_string()))?;
    if !EXPOSED_FIELDS.contains(&field) {
        return Err(FactoryError::InvalidChoice(field_id.as_str().to_string()));
    }
    let choice = choice_id
        .as_str()
        .parse::<GbcSettingChoice>()
        .map_err(|_| FactoryError::InvalidChoice(choice_id.as_str().to_string()))?;
    let settings = view
        .system_config
        .as_deref_mut()
        .and_then(|value| value.downcast_mut::<GbcSettings>())
        .ok_or(FactoryError::InvalidSettings)?;
    match (field, choice) {
        (GbcSettingField::SystemHardwareModel, GbcSettingChoice::Dmg0) => {
            settings.core.hardware_model = HardwareModel::Dmg0;
        }
        (GbcSettingField::SystemHardwareModel, GbcSettingChoice::Dmg) => {
            settings.core.hardware_model = HardwareModel::Dmg;
        }
        (GbcSettingField::SystemHardwareModel, GbcSettingChoice::CgbC) => {
            settings.core.hardware_model = HardwareModel::CgbC;
        }
        (GbcSettingField::SystemHardwareModel, GbcSettingChoice::CgbD) => {
            settings.core.hardware_model = HardwareModel::CgbD;
        }
        (GbcSettingField::SystemHardwareModel, GbcSettingChoice::Agb) => {
            settings.core.hardware_model = HardwareModel::Agb;
        }
        (GbcSettingField::SystemRtcSync, GbcSettingChoice::Off) => {
            settings.core.rtc_sync = RtcSyncMode::Off;
        }
        (GbcSettingField::SystemRtcSync, GbcSettingChoice::SystemTime) => {
            settings.core.rtc_sync = RtcSyncMode::SystemTime;
        }
        _ => return Err(FactoryError::InvalidChoice(choice_id.as_str().to_string())),
    }
    Ok(())
}

pub(crate) fn resolve_gbc_load_request(
    view: &FactorySettingsView,
    options: Box<dyn DynSystemLoadOptions>,
) -> Result<ResolvedLoadRequest, FactoryError> {
    let settings = view
        .system_config
        .as_deref()
        .and_then(|value| value.downcast_ref::<GbcSettings>())
        .ok_or(FactoryError::InvalidSettings)?;
    let options = options
        .into_inner::<GbcLoadOptions>()
        .map_err(|_| FactoryError::Resolve("failed to downcast GBC load options".to_string()))?;
    let rtc_sync = match settings.core.rtc_sync {
        RtcSyncMode::Off => RtcSyncPolicy::Off,
        RtcSyncMode::SystemTime => RtcSyncPolicy::SystemTime,
    };
    Ok(ResolvedLoadRequest {
        options: GbcCoreOptions {
            hardware_model: options
                .hardware_model
                .unwrap_or(settings.core.hardware_model),
            rtc_sync,
        }
        .into(),
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
            system_config: Some(Box::new(GbcSettings::default())),
        }
    }

    #[test]
    fn page_exposes_hardware_model_and_rtc() {
        let page = gbc_settings_page(&view());
        assert_eq!(page.fields.len(), 2);
    }

    #[test]
    fn applies_valid_choice_and_rejects_cross_field_choice() {
        let mut view = view();
        apply_gbc_settings_choice(
            &mut view,
            &SystemSettingsFieldId(Cow::Borrowed("system.hardware_model")),
            &SystemSettingsChoiceId(Cow::Borrowed("agb")),
        )
        .unwrap();
        let settings = view
            .system_config
            .as_deref()
            .unwrap()
            .downcast_ref::<GbcSettings>()
            .unwrap();
        assert_eq!(settings.core.hardware_model, HardwareModel::Agb);

        assert!(
            apply_gbc_settings_choice(
                &mut view,
                &SystemSettingsFieldId(Cow::Borrowed("system.rtc_sync")),
                &SystemSettingsChoiceId(Cow::Borrowed("agb")),
            )
            .is_err()
        );
    }

    #[test]
    fn cli_hardware_model_overrides_saved_setting() {
        let mut view = view();
        view.system_config
            .as_deref_mut()
            .unwrap()
            .downcast_mut::<GbcSettings>()
            .unwrap()
            .core
            .hardware_model = HardwareModel::Dmg;
        let resolved = resolve_gbc_load_request(
            &view,
            GbcLoadOptions {
                hardware_model: Some(HardwareModel::Agb),
            }
            .into(),
        )
        .unwrap();
        let options = resolved.options.downcast::<GbcCoreOptions>().unwrap();
        assert_eq!(options.hardware_model, HardwareModel::Agb);
    }
}
