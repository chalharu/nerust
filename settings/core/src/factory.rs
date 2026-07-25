use nerust_core_traits::{
    factory::{
        CoreFactory, FactoryError,
        descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId},
        settings::{FactorySettingsView, Language},
    },
    identity::SystemId,
};
// SystemDefaults is needed in scope for the return type of
// CoreFactory::as_system_defaults() → Option<&dyn SystemDefaults>.
// `#[allow]` suppresses clippy FP when the trait is only used
// through a method return type from another trait.
#[allow(unused_imports)]
use nerust_core_traits::factory::SystemDefaults;
use nerust_gui_settings::{language::AppLanguage, snapshot::SettingsSnapshot};

fn language_to_factory_lang(lang: AppLanguage) -> Language {
    match lang {
        AppLanguage::Japanese => Language::Japanese,
        AppLanguage::English => Language::English,
        _ => Language::SystemDefault,
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

#[cfg(test)]
mod tests {
    use nerust_core_traits::{
        declare_system_id,
        factory::{
            CoreFactory, CoreParts, FactoryError, SystemDefaults,
            descriptor::{
                SystemSettingsChoiceId, SystemSettingsChoiceOption, SystemSettingsFieldId,
                SystemSettingsFieldKind, SystemSettingsFieldModel, SystemSettingsPageModel,
            },
            settings::{FactorySettingsView, Language},
        },
        identity::SystemId,
    };
    use nerust_gui_settings::{
        app_state::DesktopAppState, language::AppLanguage, local::HostBackendLocalSettings,
        shared::DesktopSharedSettings, snapshot::SettingsSnapshot,
    };
    use nerust_settings_traits::SystemSettings;
    use std::sync::Arc;

    use super::{apply_settings_choice, language_to_factory_lang, resolve_label, settings_view};

    declare_system_id!(pub TestSysId, "test");

    /// Minimal factory for testing factory.rs functions.
    #[derive(Debug)]
    struct TestFactory;

    impl TestFactory {
        const FILTER_CHOICE: &'static str = "video.filter";
        const NTSC_RGB: &'static str = "ntsc_rgb";
        const NTSC_COMPOSITE: &'static str = "ntsc_composite";
    }

    impl SystemDefaults for TestFactory {
        fn default_system_settings(&self) -> Option<Box<dyn SystemSettings>> {
            Some(Box::new(TestSettings {
                filter: Self::NTSC_COMPOSITE.to_string(),
            }))
        }
        fn resolve_label(&self, label_id: &str, lang: &str) -> Option<String> {
            match (label_id, lang) {
                ("nes.video.filter", "ja") => Some("フィルター".into()),
                ("nes.video.filter", _) => Some("Filter".into()),
                ("ntsc_rgb", _) => Some("NTSC RGB".into()),
                ("ntsc_composite", _) => Some("NTSC Composite".into()),
                _ => None,
            }
        }
    }

    impl CoreFactory for TestFactory {
        fn system_id(&self) -> Box<dyn SystemId> {
            Box::new(TestSysId)
        }
        fn display_name(&self) -> &'static str {
            "Test"
        }
        fn as_system_defaults(&self) -> Option<&dyn SystemDefaults> {
            Some(self)
        }
        fn settings_page(&self, view: &FactorySettingsView) -> SystemSettingsPageModel {
            let current: String = view
                .system_config
                .as_ref()
                .and_then(|c| c.downcast_ref::<TestSettings>())
                .map(|s| s.filter.clone())
                .unwrap_or_else(|| Self::NTSC_COMPOSITE.to_string());
            SystemSettingsPageModel {
                fields: Arc::new([SystemSettingsFieldModel {
                    id: SystemSettingsFieldId(Self::FILTER_CHOICE.into()),
                    label_id: "nes.video.filter",
                    kind: SystemSettingsFieldKind::Choice {
                        selected: SystemSettingsChoiceId(current.clone().into()),
                        options: Arc::new([
                            SystemSettingsChoiceOption {
                                id: SystemSettingsChoiceId(Self::NTSC_RGB.into()),
                                label_id: "ntsc_rgb",
                            },
                            SystemSettingsChoiceOption {
                                id: SystemSettingsChoiceId(Self::NTSC_COMPOSITE.into()),
                                label_id: "ntsc_composite",
                            },
                        ]),
                    },
                }]),
            }
        }
        fn apply_settings_choice(
            &self,
            view: &mut FactorySettingsView,
            _field: &SystemSettingsFieldId,
            choice: &SystemSettingsChoiceId,
        ) -> Result<(), FactoryError> {
            let mut settings: Box<TestSettings> = view
                .system_config
                .take()
                .and_then(|c| c.downcast::<TestSettings>().ok())
                .unwrap_or_else(|| {
                    Box::new(TestSettings {
                        filter: Self::NTSC_COMPOSITE.to_string(),
                    })
                });
            settings.filter = choice.0.to_string();
            view.system_config = Some(settings);
            Ok(())
        }
        fn probe_media(&self, _: &nerust_core_traits::factory::load::MediaObject) -> bool {
            false
        }
        fn create_core_and_adapter_with_assignments(
            &self,
            _: &FactorySettingsView,
            _: Box<dyn nerust_core_traits::audio::AudioBackend>,
            _: &nerust_input_traits::InputAssignments,
        ) -> Result<CoreParts, FactoryError> {
            unreachable!()
        }
        fn resolve_load_request(
            &self,
            _: &FactorySettingsView,
            _: Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptions>,
        ) -> Result<nerust_core_traits::factory::load::ResolvedLoadRequest, FactoryError> {
            unreachable!()
        }
        fn default_load_options(
            &self,
        ) -> Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptions> {
            unreachable!()
        }
        fn input_system_factory(&self) -> &dyn nerust_input_traits::InputSystemFactory {
            unreachable!()
        }
        fn load_options_schema(
            &self,
        ) -> Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptionsSchema> {
            unreachable!()
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub struct TestSettings {
        filter: String,
    }

    #[typetag::serde(name = "test_settings")]
    impl SystemSettings for TestSettings {
        fn requires_live_session_rebuild(&self, _other: &dyn SystemSettings) -> bool {
            false
        }
    }

    #[track_caller]
    fn snapshot_without_system() -> SettingsSnapshot {
        SettingsSnapshot {
            shared: DesktopSharedSettings::default(),
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        }
    }

    #[test]
    fn language_to_factory_lang_mapping() {
        assert_eq!(
            language_to_factory_lang(AppLanguage::Japanese),
            Language::Japanese
        );
        assert_eq!(
            language_to_factory_lang(AppLanguage::English),
            Language::English
        );
        assert_eq!(
            language_to_factory_lang(AppLanguage::SystemDefault),
            Language::SystemDefault
        );
    }

    #[test]
    fn settings_view_empty_snapshot() {
        let view = settings_view(&snapshot_without_system(), &TestSysId);
        assert!(view.system_config.is_none());
    }

    #[test]
    fn settings_view_uses_snapshot_language() {
        let mut snapshot = snapshot_without_system();
        snapshot.shared.general.language = AppLanguage::Japanese;
        let view = settings_view(&snapshot, &TestSysId);
        assert_eq!(view.language, Language::Japanese);
    }

    #[test]
    fn resolve_label_delegates_to_factory() {
        let factory = TestFactory;
        let label = resolve_label("nes.video.filter", AppLanguage::English, &factory);
        assert_eq!(label, "Filter");
    }

    #[test]
    fn resolve_label_uses_japanese_when_specified() {
        let factory = TestFactory;
        let label = resolve_label("nes.video.filter", AppLanguage::Japanese, &factory);
        assert_eq!(label, "フィルター");
    }

    #[test]
    fn resolve_label_falls_back_to_raw_id_when_factory_returns_none() {
        let factory = TestFactory;
        let label = resolve_label("some.unknown.label", AppLanguage::English, &factory);
        assert_eq!(label, "some.unknown.label");
    }

    #[test]
    fn apply_settings_choice_seeds_default_on_missing_system_settings() {
        let factory = TestFactory;
        let mut snapshot = snapshot_without_system();
        // Snapshot has no entry for TestSysId

        apply_settings_choice(
            &factory,
            &mut snapshot,
            &SystemSettingsFieldId(TestFactory::FILTER_CHOICE.into()),
            &SystemSettingsChoiceId(TestFactory::NTSC_RGB.into()),
        )
        .unwrap();

        // After apply, the snapshot should have the system settings entry
        let sid: &dyn SystemId = &TestSysId;
        let stored = snapshot.shared.systems.get(sid);
        assert!(
            stored.is_some(),
            "system settings should be seeded from defaults"
        );
        let stored = stored.unwrap().downcast_ref::<TestSettings>().unwrap();
        assert_eq!(stored.filter, TestFactory::NTSC_RGB);
    }

    #[test]
    fn apply_settings_choice_updates_existing_settings() {
        let factory = TestFactory;
        let mut snapshot = snapshot_without_system();
        snapshot.shared.systems.insert(
            Box::new(TestSysId),
            Box::new(TestSettings {
                filter: "ntsc_composite".into(),
            }),
        );

        apply_settings_choice(
            &factory,
            &mut snapshot,
            &SystemSettingsFieldId(TestFactory::FILTER_CHOICE.into()),
            &SystemSettingsChoiceId(TestFactory::NTSC_RGB.into()),
        )
        .unwrap();

        let stored = snapshot
            .shared
            .systems
            .get(&TestSysId as &dyn SystemId)
            .unwrap();
        let stored = stored.downcast_ref::<TestSettings>().unwrap();
        assert_eq!(stored.filter, "ntsc_rgb");
    }
}
