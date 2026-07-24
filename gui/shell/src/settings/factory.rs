use nerust_core_traits::{
    factory::{
        CoreFactory,
        settings::{FactorySettingsView, Language},
    },
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

pub fn resolve_label(
    label_id: &str,
    language: nerust_gui_settings::language::AppLanguage,
    factory: &dyn CoreFactory,
) -> String {
    let lang = match language {
        nerust_gui_settings::language::AppLanguage::Japanese => "ja",
        _ => "en",
    };
    factory
        .resolve_label(label_id, lang)
        .unwrap_or_else(|| label_id.to_string())
}
