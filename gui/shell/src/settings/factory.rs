use nerust_core_traits::{
    factory::settings::{FactorySettingsView, Language},
    identity::SystemId,
};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_nes_settings::NesSettings;
use nerust_settings_traits::SystemSettings as SystemSettingsTrait;

fn system_settings_to_bytes(s: &dyn SystemSettingsTrait) -> Vec<u8> {
    let any: &dyn std::any::Any = s;
    let Some(nes) = any.downcast_ref::<NesSettings>() else {
        return Vec::new();
    };
    bytes_or_fallback(rmp_serde::to_vec(nes).map_err(|e| e.to_string()))
}

fn bytes_or_fallback(result: Result<Vec<u8>, String>) -> Vec<u8> {
    match result {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn!("NesSettings serialization failed, settings change may not apply: {e}");
            Vec::new()
        }
    }
}

fn system_settings_from_bytes(bytes: &[u8]) -> Option<Box<dyn SystemSettingsTrait>> {
    let nes = rmp_serde::from_slice::<NesSettings>(bytes).ok()?;
    Some(Box::new(nes))
}

pub fn settings_view(snapshot: &SettingsSnapshot, system_id: &SystemId) -> FactorySettingsView {
    let language = match snapshot.shared.general.language {
        nerust_gui_settings::language::AppLanguage::Japanese => Language::Japanese,
        nerust_gui_settings::language::AppLanguage::English => Language::English,
        _ => Language::SystemDefault,
    };
    let system_config_bytes = snapshot
        .shared
        .systems
        .get(system_id)
        .map(|s| system_settings_to_bytes(&**s))
        .unwrap_or_default();
    FactorySettingsView {
        language,
        system_config_bytes,
    }
}

pub fn apply_settings_choice(
    factory: &dyn nerust_core_traits::factory::CoreFactory,
    snapshot: &mut SettingsSnapshot,
    field: &nerust_core_traits::factory::descriptor::SystemSettingsFieldId,
    choice: &nerust_core_traits::factory::descriptor::SystemSettingsChoiceId,
) -> Result<(), nerust_core_traits::factory::FactoryError> {
    let system_id = factory.system_id();
    let mut view = settings_view(snapshot, &system_id);
    factory.apply_settings_choice(&mut view, field, choice)?;
    if let Some(settings) = system_settings_from_bytes(&view.system_config_bytes) {
        snapshot.shared.systems.insert(system_id, settings);
    }
    Ok(())
}

fn resolve_nes_label(
    label_id: &str,
    language: nerust_gui_settings::language::AppLanguage,
) -> String {
    use nerust_gui_settings::language::AppLanguage;
    let localized = |en: &str, ja: &str| -> String {
        match language {
            AppLanguage::Japanese => ja.to_string(),
            _ => en.to_string(),
        }
    };
    match label_id {
        "nes.video.filter" => localized("Filter", "フィルター"),
        "nes.filter.none" => localized("None", "なし"),
        "nes.filter.ntsc_composite" => localized("NTSC Composite", "NTSC コンポジット"),
        "nes.filter.ntsc_svideo" => localized("NTSC S-Video", "NTSC S-ビデオ"),
        "nes.filter.ntsc_rgb" => localized("NTSC RGB", "NTSC RGB"),
        "nes.core.mmc3_irq_variant" => localized("MMC3 IRQ Variant", "MMC3 IRQ バリアント"),
        "nes.mmc3.auto" => localized("Auto", "自動"),
        "nes.mmc3.sharp" => localized("Sharp", "Sharp"),
        "nes.mmc3.nec" => localized("Nec", "Nec"),
        _ => label_id.to_string(),
    }
}

pub fn resolve_label(
    label_id: &str,
    language: nerust_gui_settings::language::AppLanguage,
) -> String {
    if label_id.starts_with("nes.") {
        return resolve_nes_label(label_id, language);
    }
    label_id.to_string()
}

#[cfg(test)]
mod tests {

    use nerust_nes_settings::{Mmc3IrqVariant, NesSettings, NesVideoFilter};

    use super::{bytes_or_fallback, system_settings_from_bytes, system_settings_to_bytes};

    #[test]
    fn settings_round_trip_preserves_filter_value() {
        let nes = Box::new(NesSettings {
            video: nerust_nes_settings::NesVideoSettings {
                filter: NesVideoFilter::NtscRgb,
            },
            core: nerust_nes_settings::NesCoreSettings {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Sharp),
            },
        }) as Box<dyn super::SystemSettingsTrait>;
        let bytes = system_settings_to_bytes(&*nes);
        assert!(!bytes.is_empty(), "serialized bytes should not be empty");

        let restored = system_settings_from_bytes(&bytes).unwrap();
        let restored_nes = {
            let any: &dyn std::any::Any = &*restored;
            any.downcast_ref::<NesSettings>().unwrap()
        };
        assert_eq!(restored_nes.video.filter, NesVideoFilter::NtscRgb);
        assert_eq!(
            restored_nes.core.mmc3_irq_variant,
            Some(Mmc3IrqVariant::Sharp)
        );
    }

    #[test]
    fn invalid_bytes_return_none() {
        assert!(system_settings_from_bytes(&[]).is_none());
        assert!(system_settings_from_bytes(b"garbage").is_none());
    }

    #[test]
    fn system_settings_to_returns_valid_msgpack() {
        let nes = Box::new(NesSettings {
            video: nerust_nes_settings::NesVideoSettings {
                filter: NesVideoFilter::NtscComposite,
            },
            core: nerust_nes_settings::NesCoreSettings::default(),
        }) as Box<dyn super::SystemSettingsTrait>;
        let bytes = system_settings_to_bytes(&*nes);
        assert!(!bytes.is_empty(), "serialized bytes should not be empty");

        let decoded: nerust_nes_settings::NesSettings =
            rmp_serde::from_slice(&bytes).expect("valid MessagePack");
        assert_eq!(decoded.video.filter, NesVideoFilter::NtscComposite);
    }

    #[test]
    fn serialization_fallback_returns_empty_on_error() {
        assert!(bytes_or_fallback(Err("test error".to_string())).is_empty());
    }

    #[test]
    fn serialization_fallback_passes_through_bytes() {
        assert_eq!(bytes_or_fallback(Ok(vec![1, 2, 3])), vec![1, 2, 3]);
    }
}
