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

// Safety: `Arc<dyn CoreFactory>` requires `dyn CoreFactory: Send + Sync`,
// which is enforced at construction for each concrete factory type.
// `NesFactory` and all future factories are `Send + Sync` (zero-sized or
// containing only `Send + Sync` data).
unsafe impl Send for SystemRegistry {}
unsafe impl Sync for SystemRegistry {}

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

        target.set_active_system(system_id);
        target.resume();
        Ok(())
    }
}
