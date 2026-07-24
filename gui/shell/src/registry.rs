use std::{collections::HashMap, path::Path, sync::Arc};

use nerust_core_traits::{
    factory::{
        CoreFactory,
        load::{DynSystemLoadOptions, MediaObject},
    },
    identity::SystemId,
};
use nerust_gui_runtime::rom::load_rom_path;

use crate::{
    load::{RomLoadTarget, RomLoader, RomLoaderError},
    settings::factory::settings_view,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("load options provided for unregistered system: {0}")]
    UnregisteredOptions(Box<dyn SystemId>),
    #[error("multiple systems matched the media: {0:?}")]
    AmbiguousMedia(Vec<Box<dyn SystemId>>),
}

/// Registry of all supported console systems.
///
/// Handles system auto-detection and dispatching to the correct
/// `CoreFactory`. Currently only NES is registered, but SNES/GB
/// can be added by appending to the `Vec` at construction time.
pub struct SystemRegistry {
    factories: Vec<Arc<dyn CoreFactory>>,
    by_id: HashMap<Box<dyn SystemId>, Arc<dyn CoreFactory>>,
}

impl SystemRegistry {
    pub fn new(factories: Vec<Arc<dyn CoreFactory>>) -> Self {
        let mut by_id = HashMap::with_capacity(factories.len());
        for f in &factories {
            let system_id = f.system_id();
            assert!(
                by_id.insert(system_id.clone(), Arc::clone(f)).is_none(),
                "SystemRegistry: duplicate system_id: {system_id}"
            );
        }
        Self { factories, by_id }
    }

    /// Returns all registered factories, for CLI argument augmentation.
    pub fn all(&self) -> &[Arc<dyn CoreFactory>] {
        &self.factories
    }

    /// Returns the sole factory that handles the given media.
    ///
    /// Returns `Ok(None)` when no factory matches and an error when multiple
    /// factories claim the same media, avoiding registration-order dispatch.
    pub fn detect(
        &self,
        media: &MediaObject,
    ) -> Result<Option<&Arc<dyn CoreFactory>>, RegistryError> {
        let mut matches = self.factories.iter().filter(|f| f.probe_media(media));
        let Some(first) = matches.next() else {
            return Ok(None);
        };
        let Some(second) = matches.next() else {
            return Ok(Some(first));
        };
        let mut system_ids = vec![first.system_id(), second.system_id()];
        system_ids.extend(matches.map(|factory| factory.system_id()));
        Err(RegistryError::AmbiguousMedia(system_ids))
    }

    /// Finds a factory by its system ID. O(1) lookup.
    pub fn find_by_id(&self, id: &dyn SystemId) -> Option<&Arc<dyn CoreFactory>> {
        self.by_id.get(id)
    }

    /// Creates a `RomLoader` that auto-detects the system for each load.
    ///
    /// `pending_options` maps each system ID to CLI-provided load options.
    /// Each option is consumed on the first
    /// load of the corresponding system; subsequent loads fall back to
    /// `RomLoadTarget::default_load_options()`.
    pub fn create_loader(
        self: &Arc<Self>,
        pending_options: HashMap<Box<dyn SystemId>, Box<dyn DynSystemLoadOptions>>,
    ) -> Result<Box<dyn RomLoader>, RegistryError> {
        if let Some(system_id) = pending_options
            .keys()
            .find(|system_id| !self.by_id.contains_key(system_id.as_ref()))
        {
            return Err(RegistryError::UnregisteredOptions(system_id.clone()));
        }
        let pending_options = pending_options
            .into_iter()
            .map(|(system_id, options)| (system_id, Some(options)))
            .collect();
        Ok(Box::new(RegistryRomLoader {
            registry: Arc::clone(self),
            pending_options,
        }))
    }
}

/// `RomLoader` that dispatches to the correct `CoreFactory` based on
/// ROM auto-detection via `probe_media()`.
struct RegistryRomLoader {
    registry: Arc<SystemRegistry>,
    pending_options: HashMap<Box<dyn SystemId>, Option<Box<dyn DynSystemLoadOptions>>>,
}

impl RomLoader for RegistryRomLoader {
    fn load_rom(
        &mut self,
        path: &Path,
        target: &mut dyn RomLoadTarget,
    ) -> Result<(), RomLoaderError> {
        let loaded = load_rom_path(path).map_err(|e| RomLoaderError::Io(e.to_string()))?;
        let (rom_path, data) = loaded.into_parts();
        let media = MediaObject::new(Some(rom_path), data);

        let factory = self
            .registry
            .detect(&media)
            .map_err(|error| RomLoaderError::Detect(error.to_string()))?
            .ok_or_else(|| RomLoaderError::Detect("unsupported ROM format".to_string()))?;

        let system_id = factory.system_id();

        // Notify the target BEFORE loading so it can rebuild the
        // EmuCore with the correct factory if the system changed.
        target
            .set_active_system(system_id.as_ref())
            .map_err(|e| RomLoaderError::Detect(e.to_string()))?;

        let view = settings_view(target.settings_snapshot(), system_id.as_ref());
        let options = self
            .pending_options
            .get_mut(&system_id)
            .and_then(|opt| opt.take())
            .unwrap_or_else(|| {
                target
                    .default_load_options()
                    .unwrap_or_else(|| factory.default_load_options())
            });

        let resolved = factory
            .resolve_load_request(&view, options)
            .map_err(|e| RomLoaderError::Resolve(e.to_string()))?;
        target.load_resolved(media, resolved)?;

        target.resume();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nerust_core_traits::{
        declare_system_id,
        factory::{
            CoreFactory, CoreParts, FactoryError,
            descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel},
            load::{
                DynSystemLoadOptions, DynSystemLoadOptionsSchema, MediaObject, ResolvedLoadRequest,
                SystemLoadOptions,
            },
            settings::FactorySettingsView,
        },
        identity::SystemId,
    };
    use nerust_input_traits::{InputAssignments, InputSystemFactory};

    use super::*;

    declare_system_id!(DummySystemId, "dummy");
    declare_system_id!(DummyOtherSystemId, "other");

    #[derive(Debug, Clone)]
    struct StubFactory;

    impl CoreFactory for StubFactory {
        fn system_id(&self) -> Box<dyn SystemId> {
            Box::new(DummySystemId)
        }
        fn display_name(&self) -> &'static str {
            "Stub"
        }
        fn probe_media(&self, _media: &MediaObject) -> bool {
            false
        }
        fn settings_page(&self, _view: &FactorySettingsView) -> SystemSettingsPageModel {
            SystemSettingsPageModel {
                fields: Arc::new([]),
            }
        }
        fn apply_settings_choice(
            &self,
            _view: &mut FactorySettingsView,
            _field: &SystemSettingsFieldId,
            _choice: &SystemSettingsChoiceId,
        ) -> Result<(), FactoryError> {
            Ok(())
        }
        fn resolve_load_request(
            &self,
            _view: &FactorySettingsView,
            _options: Box<dyn DynSystemLoadOptions>,
        ) -> Result<ResolvedLoadRequest, FactoryError> {
            Ok(ResolvedLoadRequest {
                options: Box::<NoopCoreOptions>::default(),
            })
        }
        fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
            NoopSystemLoadOptions.into()
        }
        fn create_core_and_adapter_with_assignments(
            &self,
            _view: &FactorySettingsView,
            _speaker: Box<dyn nerust_core_traits::audio::AudioBackend>,
            _assignments: &InputAssignments,
        ) -> Result<CoreParts, FactoryError> {
            unreachable!()
        }
        fn input_system_factory(&self) -> &dyn InputSystemFactory {
            unreachable!()
        }
        fn load_options_schema(&self) -> Box<dyn DynSystemLoadOptionsSchema> {
            unreachable!()
        }
    }

    #[derive(Debug, Clone)]
    struct MatchingStubFactory(Box<dyn SystemId>);

    impl CoreFactory for MatchingStubFactory {
        fn system_id(&self) -> Box<dyn SystemId> {
            self.0.clone()
        }
        fn display_name(&self) -> &'static str {
            "Matched"
        }
        fn probe_media(&self, _media: &MediaObject) -> bool {
            true
        }
        fn settings_page(&self, _view: &FactorySettingsView) -> SystemSettingsPageModel {
            SystemSettingsPageModel {
                fields: Arc::new([]),
            }
        }
        fn apply_settings_choice(
            &self,
            _view: &mut FactorySettingsView,
            _field: &SystemSettingsFieldId,
            _choice: &SystemSettingsChoiceId,
        ) -> Result<(), FactoryError> {
            Ok(())
        }
        fn resolve_load_request(
            &self,
            _view: &FactorySettingsView,
            _options: Box<dyn DynSystemLoadOptions>,
        ) -> Result<ResolvedLoadRequest, FactoryError> {
            Ok(ResolvedLoadRequest {
                options: Box::<NoopCoreOptions>::default(),
            })
        }
        fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
            NoopSystemLoadOptions.into()
        }
        fn create_core_and_adapter_with_assignments(
            &self,
            _view: &FactorySettingsView,
            _speaker: Box<dyn nerust_core_traits::audio::AudioBackend>,
            _assignments: &InputAssignments,
        ) -> Result<CoreParts, FactoryError> {
            unreachable!()
        }
        fn input_system_factory(&self) -> &dyn InputSystemFactory {
            unreachable!()
        }
        fn load_options_schema(&self) -> Box<dyn DynSystemLoadOptionsSchema> {
            unreachable!()
        }
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    struct NoopCoreOptions;

    impl nerust_core_traits::CoreOptions for NoopCoreOptions {}

    #[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
    struct NoopSystemLoadOptions;

    impl SystemLoadOptions for NoopSystemLoadOptions {}

    fn stub_factory() -> Arc<dyn CoreFactory> {
        Arc::new(StubFactory)
    }

    #[test]
    fn empty_registry_all_returns_empty_slice() {
        let registry = SystemRegistry::new(vec![]);
        assert_eq!(registry.all().len(), 0);
    }

    #[test]
    fn all_preserves_registration_order() {
        let a = stub_factory();
        let b: Arc<dyn CoreFactory> = Arc::new(MatchingStubFactory(Box::new(DummyOtherSystemId)));
        let registry = SystemRegistry::new(vec![a.clone(), b.clone()]);
        assert_eq!(registry.all().len(), 2);
        assert_eq!(registry.all()[0].system_id(), a.system_id());
        assert_eq!(registry.all()[1].system_id(), b.system_id());
    }

    #[test]
    fn find_by_id_returns_factory() {
        let factory = stub_factory();
        let id = factory.system_id();
        let registry = SystemRegistry::new(vec![factory.clone()]);
        assert!(registry.find_by_id(id.as_ref()).is_some());
        assert!(registry.find_by_id(&DummyOtherSystemId).is_none());
    }

    #[test]
    #[should_panic(expected = "duplicate system_id")]
    fn duplicate_system_id_is_rejected() {
        SystemRegistry::new(vec![stub_factory(), stub_factory()]);
    }

    #[test]
    fn detect_returns_none_when_no_factory_matches() {
        let registry = SystemRegistry::new(vec![stub_factory()]);
        let media = MediaObject::new(Some("game.sfc".into()), vec![]);
        assert!(registry.detect(&media).unwrap().is_none());
    }

    #[test]
    fn detect_returns_matching_factory() {
        let fallback = stub_factory();
        let matched = Arc::new(MatchingStubFactory(Box::new(DummyOtherSystemId)));
        let matched_id = matched.system_id();
        let registry = SystemRegistry::new(vec![fallback, matched]);
        let media = MediaObject::new(Some("game.nes".into()), vec![]);
        assert_eq!(
            registry.detect(&media).unwrap().unwrap().system_id(),
            matched_id
        );
    }

    #[test]
    fn detect_rejects_ambiguous_media() {
        let first: Arc<dyn CoreFactory> = Arc::new(MatchingStubFactory(Box::new(DummySystemId)));
        let second: Arc<dyn CoreFactory> =
            Arc::new(MatchingStubFactory(Box::new(DummyOtherSystemId)));
        let registry = SystemRegistry::new(vec![first, second]);
        let media = MediaObject::new(Some("game.rom".into()), vec![]);

        assert!(matches!(
            registry.detect(&media),
            Err(RegistryError::AmbiguousMedia(ids))
                if ids == vec![Box::new(DummySystemId) as Box<dyn SystemId>, Box::new(DummyOtherSystemId)]
        ));
    }

    #[test]
    fn create_loader_accepts_options() {
        let factory = stub_factory();
        let registry = Arc::new(SystemRegistry::new(vec![factory.clone()]));
        let opts = factory.default_load_options();
        let _loader = registry
            .create_loader(HashMap::from([(factory.system_id(), opts)]))
            .unwrap();
    }

    #[test]
    fn create_loader_rejects_options_for_unregistered_system() {
        let registry = Arc::new(SystemRegistry::new(vec![stub_factory()]));
        let options = NoopSystemLoadOptions.into();

        assert!(matches!(
            registry.create_loader(HashMap::from([(Box::new(DummyOtherSystemId) as Box<_>, options)])),
            Err(RegistryError::UnregisteredOptions(id)) if id.as_ref() == &DummyOtherSystemId as &dyn SystemId
        ));
    }
}
