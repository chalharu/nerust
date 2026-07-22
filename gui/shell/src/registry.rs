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

/// Registry of all supported console systems.
///
/// Handles system auto-detection and dispatching to the correct
/// `CoreFactory`. Currently only NES is registered, but SNES/GB
/// can be added by appending to the `Vec` at construction time.
pub struct SystemRegistry {
    factories: Vec<Arc<dyn CoreFactory>>,
}

impl SystemRegistry {
    pub fn new(factories: Vec<Arc<dyn CoreFactory>>) -> Self {
        assert!(!factories.is_empty(), "at least one CoreFactory required");
        Self { factories }
    }

    /// Returns all registered factories, for CLI argument augmentation.
    pub fn all(&self) -> &[Arc<dyn CoreFactory>] {
        &self.factories
    }

    /// Returns the primary (first registered) factory.
    /// Used as the default system when no ROM is loaded.
    pub fn primary(&self) -> &Arc<dyn CoreFactory> {
        &self.factories[0]
    }

    /// Returns the factory that handles the given media.
    /// Falls back to the primary factory if no match.
    pub fn detect(&self, media: &MediaObject) -> &Arc<dyn CoreFactory> {
        self.factories
            .iter()
            .find(|f| f.probe_media(media))
            .unwrap_or_else(|| self.primary())
    }

    /// Finds a factory by its system ID.
    pub fn find_by_id(&self, id: &SystemId) -> Option<&Arc<dyn CoreFactory>> {
        self.factories.iter().find(|f| f.system_id() == *id)
    }

    /// Creates a `RomLoader` that auto-detects the system for each load.
    ///
    /// `pending_options` maps each factory (by registration order) to
    /// CLI-provided load options. Each option is consumed on the first
    /// load of the corresponding system; subsequent loads fall back to
    /// `RomLoadTarget::default_load_options()`.
    pub fn create_loader(
        &self,
        pending_options: Vec<Box<dyn DynSystemLoadOptions>>,
    ) -> Box<dyn RomLoader> {
        let opt_by_id: HashMap<SystemId, Option<Box<dyn DynSystemLoadOptions>>> = self
            .factories
            .iter()
            .zip(pending_options.into_iter().map(Some))
            .map(|(f, opt)| (f.system_id(), opt))
            .collect();
        Box::new(RegistryRomLoader {
            registry: self.factories.clone(),
            pending_options: opt_by_id,
        })
    }
}

/// `RomLoader` that dispatches to the correct `CoreFactory` based on
/// ROM auto-detection via `probe_media()`.
struct RegistryRomLoader {
    registry: Vec<Arc<dyn CoreFactory>>,
    pending_options: HashMap<SystemId, Option<Box<dyn DynSystemLoadOptions>>>,
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
            .iter()
            .find(|f| f.probe_media(&media))
            .ok_or_else(|| RomLoaderError::Io("unsupported ROM format".to_string()))?;

        let system_id = factory.system_id();

        // Notify the target BEFORE loading so it can rebuild the
        // EmuCore with the correct factory if the system changed.
        target.set_active_system(system_id);

        let view = settings_view(target.settings_snapshot(), &system_id);
        let options = self
            .pending_options
            .get_mut(&system_id)
            .and_then(|opt| opt.take())
            .unwrap_or_else(|| target.default_load_options());

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

    #[derive(Debug, Clone)]
    struct StubFactory;

    impl CoreFactory for StubFactory {
        fn system_id(&self) -> SystemId {
            SystemId::new("nes")
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
    #[should_panic(expected = "at least one CoreFactory required")]
    fn new_panics_on_empty_vec() {
        SystemRegistry::new(vec![]);
    }

    #[test]
    fn primary_returns_first_registered() {
        let a = stub_factory();
        let b = stub_factory();
        let registry = SystemRegistry::new(vec![a.clone(), b.clone()]);
        assert_eq!(registry.primary().system_id(), a.system_id());
        assert_eq!(registry.all().len(), 2);
    }

    #[test]
    fn find_by_id_returns_factory() {
        let factory = stub_factory();
        let id = factory.system_id();
        let registry = SystemRegistry::new(vec![factory.clone(), stub_factory()]);
        assert!(registry.find_by_id(&id).is_some());
        assert!(registry.find_by_id(&SystemId::new("snes")).is_none());
    }

    #[test]
    fn detect_falls_back_to_primary() {
        let a = stub_factory();
        let registry = SystemRegistry::new(vec![a.clone(), stub_factory()]);
        let media = MediaObject::new(Some("game.sfc".into()), vec![]);
        assert_eq!(registry.detect(&media).system_id(), a.system_id());
    }

    #[test]
    fn create_loader_accepts_options() {
        let factory = stub_factory();
        let registry = SystemRegistry::new(vec![factory.clone()]);
        let opts = factory.default_load_options();
        let _loader = registry.create_loader(vec![opts]);
    }
}
