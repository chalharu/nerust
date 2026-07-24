use nerust_core_traits::{
    factory::{
        CoreFactory,
        settings::{FactorySettingsView, Language},
    },
    identity::SystemId,
};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_settings::language::AppLanguage;

fn language_to_factory_lang(lang: AppLanguage) -> Language {
    match lang {
        AppLanguage::Japanese => Language::Japanese,
        AppLanguage::English => Language::English,
        _ => Language::SystemDefault,
    }
}

fn language_to_str(lang: AppLanguage) -> &'static str {
    match lang {
        AppLanguage::Japanese => "ja",
        _ => "en",
    }
}

pub fn settings_view(snapshot: &SettingsSnapshot, system_id: &SystemId) -> FactorySettingsView {
    let language = language_to_factory_lang(snapshot.shared.general.language);
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

pub fn resolve_label(label_id: &str, language: AppLanguage, factory: &dyn CoreFactory) -> String {
    factory
        .resolve_label(label_id, language_to_str(language))
        .unwrap_or_else(|| label_id.to_string())
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::{
        factory::{
            CoreFactory, FactoryError,
            descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel},
            load::{
                DynSystemLoadOptions, DynSystemLoadOptionsSchema, MediaObject, ResolvedLoadRequest,
            },
            settings::FactorySettingsView,
        },
        identity::SystemId,
    };
    use nerust_gui_settings::language::AppLanguage;
    use std::sync::Arc;

    use super::*;

    struct LabelFactory {
        labels: Vec<(&'static str, &'static str)>,
    }
    impl CoreFactory for LabelFactory {
        fn system_id(&self) -> SystemId {
            SystemId::new("test")
        }
        fn display_name(&self) -> &'static str {
            "Test"
        }
        fn probe_media(&self, _: &MediaObject) -> bool {
            false
        }
        fn settings_page(&self, _: &FactorySettingsView) -> SystemSettingsPageModel {
            SystemSettingsPageModel {
                fields: Arc::new([]),
            }
        }
        fn apply_settings_choice(
            &self,
            _: &mut FactorySettingsView,
            _: &SystemSettingsFieldId,
            _: &SystemSettingsChoiceId,
        ) -> Result<(), FactoryError> {
            Ok(())
        }
        fn resolve_load_request(
            &self,
            _: &FactorySettingsView,
            _: Box<dyn DynSystemLoadOptions>,
        ) -> Result<ResolvedLoadRequest, FactoryError> {
            unimplemented!()
        }
        fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
            unimplemented!()
        }
        fn load_options_schema(&self) -> Box<dyn DynSystemLoadOptionsSchema> {
            unimplemented!()
        }
        fn create_core_and_adapter_with_assignments(
            &self,
            _: &FactorySettingsView,
            _: Box<dyn nerust_core_traits::audio::AudioBackend>,
            _: &nerust_input_traits::InputAssignments,
        ) -> Result<nerust_core_traits::factory::CoreParts, FactoryError> {
            unimplemented!()
        }
        fn input_system_factory(&self) -> &dyn nerust_input_traits::InputSystemFactory {
            unimplemented!()
        }
        fn resolve_label(&self, label_id: &str, _language: &str) -> Option<String> {
            self.labels
                .iter()
                .find(|(id, _)| *id == label_id)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn resolve_label_delegates_to_factory() {
        let factory = LabelFactory {
            labels: vec![("test.key", "Nice Label")],
        };
        let result = resolve_label("test.key", AppLanguage::English, &factory);
        assert_eq!(result, "Nice Label");
    }

    #[test]
    fn resolve_label_falls_back_to_raw_id() {
        let factory = LabelFactory { labels: vec![] };
        let result = resolve_label("unknown.label", AppLanguage::English, &factory);
        assert_eq!(result, "unknown.label");
    }

    #[test]
    fn resolve_label_passes_language_to_factory() {
        let factory = LabelFactory {
            labels: vec![("test.lang", "ja:日本語")],
        };
        let result = resolve_label("test.lang", AppLanguage::Japanese, &factory);
        assert_eq!(result, "ja:日本語");
    }
}
