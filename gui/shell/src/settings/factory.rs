use nerust_core_traits::{
    factory::settings::{FactorySettingsView, Language},
    identity::SystemId,
};
use nerust_gui_runtime::settings::SettingsSnapshot;

pub fn settings_view(snapshot: &SettingsSnapshot, system_id: &SystemId) -> FactorySettingsView {
    let language = match snapshot.shared.general.language {
        nerust_gui_settings::language::AppLanguage::Japanese => Language::Japanese,
        nerust_gui_settings::language::AppLanguage::English => Language::English,
        _ => Language::SystemDefault,
    };
    let system_config = snapshot.shared.systems.get(system_id).cloned();
    FactorySettingsView {
        language,
        system_config,
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
    if let Some(settings) = view.system_config {
        snapshot.shared.systems.insert(system_id, settings);
    }
    Ok(())
}

fn resolve_nes_label(
    label_id: &str,
    language: nerust_gui_settings::language::AppLanguage,
) -> Option<String> {
    use nerust_gui_settings::language::AppLanguage;
    let localized = |en: &str, ja: &str| -> String {
        match language {
            AppLanguage::Japanese => ja.to_string(),
            _ => en.to_string(),
        }
    };
    match label_id {
        "nes.video.filter" => Some(localized("Filter", "フィルター")),
        "nes.filter.none" => Some(localized("None", "なし")),
        "nes.filter.ntsc_composite" => Some(localized("NTSC Composite", "NTSC コンポジット")),
        "nes.filter.ntsc_svideo" => Some(localized("NTSC S-Video", "NTSC S-ビデオ")),
        "nes.filter.ntsc_rgb" => Some(localized("NTSC RGB", "NTSC RGB")),
        "nes.core.mmc3_irq_variant" => Some(localized("MMC3 IRQ Variant", "MMC3 IRQ バリアント")),
        "nes.mmc3.auto" => Some(localized("Auto", "自動")),
        "nes.mmc3.sharp" => Some(localized("Sharp", "Sharp")),
        "nes.mmc3.nec" => Some(localized("Nec", "Nec")),
        _ => None,
    }
}

pub fn resolve_label(
    label_id: &str,
    language: nerust_gui_settings::language::AppLanguage,
) -> String {
    // Per-system label resolvers return Some(translated) or None.
    // The first match wins; add new resolvers before the final fallback.
    resolve_nes_label(label_id, language).unwrap_or_else(|| label_id.to_string())
}
