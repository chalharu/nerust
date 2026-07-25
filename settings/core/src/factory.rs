use nerust_core_traits::{
    factory::{
        CoreFactory, FactoryError,
        descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId},
        settings::FactorySettingsView,
    },
    identity::SystemId,
};
use nerust_gui_settings::{language::AppLanguage, snapshot::SettingsSnapshot};

fn language_to_factory_lang(lang: AppLanguage) -> nerust_core_traits::factory::settings::Language {
    match lang {
        AppLanguage::Japanese => nerust_core_traits::factory::settings::Language::Japanese,
        AppLanguage::English => nerust_core_traits::factory::settings::Language::English,
        _ => nerust_core_traits::factory::settings::Language::SystemDefault,
    }
}

pub fn settings_view(snapshot: &SettingsSnapshot, system_id: &dyn SystemId) -> FactorySettingsView {
    let language = language_to_factory_lang(snapshot.shared.general.language);
    let system_config = snapshot.shared.systems.get(system_id).cloned();
    FactorySettingsView {
        language,
        system_config,
    }
}

pub fn apply_settings_choice(
    factory: &dyn CoreFactory,
    snapshot: &mut SettingsSnapshot,
    field: &SystemSettingsFieldId,
    choice: &SystemSettingsChoiceId,
) -> Result<(), FactoryError> {
    let system_id = factory.system_id();
    let mut view = settings_view(snapshot, system_id.as_ref());
    if view.system_config.is_none() {
        view.system_config = factory
            .as_system_defaults()
            .and_then(|defaults| defaults.default_system_settings());
    }
    factory.apply_settings_choice(&mut view, field, choice)?;
    if let Some(settings) = view.system_config {
        snapshot.shared.systems.insert(system_id, settings);
    }
    Ok(())
}

pub fn resolve_label(label_id: &str, language: AppLanguage, factory: &dyn CoreFactory) -> String {
    factory
        .as_system_defaults()
        .and_then(|d| {
            d.resolve_label(
                label_id,
                match language {
                    AppLanguage::Japanese => "ja",
                    _ => "en",
                },
            )
        })
        .unwrap_or_else(|| label_id.to_string())
}
