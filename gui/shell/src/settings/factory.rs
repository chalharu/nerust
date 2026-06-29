use nerust_core_traits::SystemId;
use nerust_core_traits::factory::settings::{FactorySettingsView, Language};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_settings::shared::SystemSettings;

fn system_settings_to_bytes(s: &SystemSettings) -> Vec<u8> {
    match s {
        SystemSettings::Nes(nes) => rmp_serde::to_vec(nes).unwrap_or_default(),
    }
}

fn system_settings_from_bytes(bytes: &[u8]) -> Option<SystemSettings> {
    let nes = rmp_serde::from_slice::<nerust_gui_settings::nes::NesSettings>(bytes).ok()?;
    Some(SystemSettings::Nes(nes))
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
        .map(system_settings_to_bytes)
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
    // Write back system config to snapshot
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
