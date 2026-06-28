pub mod access;
pub mod commands;
pub mod input;
mod lifecycle;
pub mod metrics;
pub mod persistence;
#[cfg(test)]
mod tests;
pub mod title;

use std::{collections::BTreeSet, sync::Arc};

pub use lifecycle::WindowSize;
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_input_traits::SystemInputAdapter;
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, SettingsError, SettingsPaths, SettingsSnapshot,
    manager::SettingsManager,
};
use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_persistence::{error::PersistenceError, model::StateSlotSummary};
use nerust_render_base::{FrameBuffer, VideoRenderProfile};
use thiserror::Error;

use crate::{
    descriptor::{SystemDescriptor, SystemSettingsPageModel},
    emu_core::{EmuCore, OperationError},
    factory::{CoreFactory, FactoryError},
    load::{MediaObject, SystemLoadOptions},
    session::{metrics::ConsoleMetrics, persistence::PersistenceManager},
    settings,
};

#[derive(Debug, Clone)]
pub(super) struct LoadedMedia {
    media: MediaObject,
}

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub metrics: ConsoleMetrics,
    pub input_topology: Option<nerust_input_traits::InputTopologyDescriptor>,
    pub slots: Arc<[StateSlotSummary]>,
    pub active_slot_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardShortcut {
    Session(ShortcutAction),
    ToggleFullscreen,
}

pub struct SessionHandle {
    pub(super) descriptor: SystemDescriptor,
    pub(super) factory: Arc<dyn CoreFactory>,
    pub(super) emu_core: EmuCore,
    pub(super) input_adapter: Box<dyn SystemInputAdapter>,
    pub(super) capabilities: HostBackendCapabilities,
    pub(super) settings: SettingsManager,
    pub(super) settings_snapshot: SettingsSnapshot,
    pub(super) pressed_keys: BTreeSet<KeyboardKey>,
    pub(super) loaded_media: Option<LoadedMedia>,
    pub(super) persistence: PersistenceManager,
    pub(super) audio_registry: Arc<AudioBackendRegistry>,
}

impl SessionHandle {
    /// Create a core from settings, falling back to defaults on failure.
    fn create_core_or_default(
        factory: &Arc<dyn CoreFactory>,
        registry: &AudioBackendRegistry,
        snapshot: &SettingsSnapshot,
    ) -> (EmuCore, Box<dyn SystemInputAdapter>) {
        let speaker = settings::build_speaker(registry, &snapshot.local);
        factory
            .create_core_and_adapter(snapshot, speaker)
            .unwrap_or_else(|_| {
                log::warn!("core creation with loaded settings failed; using defaults");
                use crate::settings::defaults::seed::{
                    default_app_state, default_local_settings, default_shared_settings,
                };
                let fallback = SettingsSnapshot {
                    shared: default_shared_settings(),
                    local: default_local_settings(),
                    app_state: default_app_state(),
                };
                let fallback_speaker = settings::build_speaker(registry, &fallback.local);
                factory
                    .create_core_and_adapter(&fallback, fallback_speaker)
                    .expect("failed to create core even with default settings")
            })
    }

    fn new_inner(
        capabilities: HostBackendCapabilities,
        descriptor: SystemDescriptor,
        factory: Arc<dyn CoreFactory>,
        audio_registry: Arc<AudioBackendRegistry>,
        use_persistent: bool,
    ) -> Self {
        use crate::settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        };
        let settings = if use_persistent {
            SettingsManager::load_or_ephemeral(
                default_shared_settings(),
                default_local_settings(),
                default_app_state(),
            )
        } else {
            SettingsManager::ephemeral(
                default_shared_settings(),
                default_local_settings(),
                default_app_state(),
            )
        };
        let settings_snapshot = settings
            .snapshot()
            .expect("settings snapshot should be readable");
        let (emu_core, input_adapter) =
            Self::create_core_or_default(&factory, &audio_registry, &settings_snapshot);
        Self {
            emu_core,
            input_adapter,
            descriptor,
            factory,
            capabilities,
            settings,
            settings_snapshot,
            pressed_keys: BTreeSet::new(),
            loaded_media: None,
            persistence: PersistenceManager::new(),
            audio_registry,
        }
    }

    pub fn new(
        capabilities: HostBackendCapabilities,
        descriptor: SystemDescriptor,
        factory: Arc<dyn CoreFactory>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Self {
        Self::new_inner(capabilities, descriptor, factory, audio_registry, true)
    }

    /// Create a session with settings persisted at the given paths.
    ///
    /// On platforms where `ProjectDirs` is unavailable (e.g. Android),
    /// the frontend provides an explicit settings root instead.
    pub fn new_with_settings_paths(
        capabilities: HostBackendCapabilities,
        descriptor: SystemDescriptor,
        factory: Arc<dyn CoreFactory>,
        audio_registry: Arc<AudioBackendRegistry>,
        paths: SettingsPaths,
    ) -> Self {
        use crate::settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        };
        let defaults = SettingsSnapshot {
            shared: default_shared_settings(),
            local: default_local_settings(),
            app_state: default_app_state(),
        };
        let settings = SettingsManager::load_or_ephemeral_with_paths(
            paths,
            defaults.shared.clone(),
            defaults.local.clone(),
            defaults.app_state.clone(),
        );
        let settings_snapshot = settings
            .snapshot()
            .expect("settings snapshot should be readable");
        let (emu_core, input_adapter) =
            Self::create_core_or_default(&factory, &audio_registry, &settings_snapshot);
        Self {
            emu_core,
            input_adapter,
            descriptor,
            factory,
            capabilities,
            settings,
            settings_snapshot,
            pressed_keys: BTreeSet::new(),
            loaded_media: None,
            persistence: PersistenceManager::new(),
            audio_registry,
        }
    }

    #[cfg(test)]
    pub fn new_ephemeral(
        capabilities: HostBackendCapabilities,
        descriptor: SystemDescriptor,
        factory: Arc<dyn CoreFactory>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Self {
        Self::new_inner(capabilities, descriptor, factory, audio_registry, false)
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            metrics: self.emu_core.metrics(),
            input_topology: Some(self.descriptor.input_topology.clone()),
            slots: Arc::from(self.persistence.slots().to_vec()),
            active_slot_id: self.persistence.active_slot_id(),
        }
    }

    pub fn render_profile(&self) -> &VideoRenderProfile {
        self.emu_core.render_profile()
    }

    pub fn swap_frame_buffer(&mut self) {
        self.emu_core.swap_frame_buffer();
    }

    pub fn frame_buffer(&self) -> &FrameBuffer {
        self.emu_core.frame_buffer()
    }

    pub fn clear_display(&mut self) {
        self.emu_core.clear_display();
    }

    pub fn settings_snapshot(&self) -> &SettingsSnapshot {
        &self.settings_snapshot
    }

    pub fn settings_manager(&self) -> &SettingsManager {
        &self.settings
    }

    pub fn factory(&self) -> &dyn CoreFactory {
        &*self.factory
    }

    pub fn settings_page(&self, settings: &SettingsSnapshot) -> SystemSettingsPageModel {
        self.factory.settings_page(settings)
    }

    pub fn default_load_options(&self) -> SystemLoadOptions {
        self.factory.default_load_options()
    }

    pub fn with_frame_buffer(&self, f: &mut dyn FnMut(&[u8])) {
        f(self.emu_core.frame_buffer().as_ref());
    }
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("operation: {0}")]
    Operation(#[from] OperationError),
    #[error("settings: {0}")]
    Settings(#[from] SettingsError),
    #[error("persistence: {0}")]
    Persistence(#[from] PersistenceError),
    #[error("factory: {0}")]
    Factory(#[from] FactoryError),
}

use crate::load::{ResolvedLoadRequest, RomLoadTarget, RomLoaderError};
use crate::session::commands::SessionCommand;

impl RomLoadTarget for SessionHandle {
    fn default_load_options(&self) -> SystemLoadOptions {
        SessionHandle::default_load_options(self)
    }
    fn settings_snapshot(&self) -> &SettingsSnapshot {
        SessionHandle::settings_snapshot(self)
    }
    fn load_resolved(
        &mut self,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), RomLoaderError> {
        SessionHandle::load_resolved(self, media, resolved)
            .map_err(|e| RomLoaderError::Load(e.to_string()))
    }
    fn resume(&mut self) {
        let _ = SessionHandle::run_command(self, SessionCommand::Resume);
    }
}
