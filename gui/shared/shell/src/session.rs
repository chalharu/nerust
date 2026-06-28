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
use nerust_contract_core::input::SystemInputAdapter;
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, SettingsError, SettingsSnapshot, manager::SettingsManager,
};
use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_persistence::{error::PersistenceError, model::StateSlotSummary};
use nerust_screen_video::{FrameBuffer, VideoRenderProfile};
use thiserror::Error;

use crate::{
    descriptor::{SystemDescriptor, SystemSettingsPageModel},
    emu_core::{EmuCore, OperationError},
    factory::{CoreFactory, FactoryError},
    load::{MediaObject, SystemLoadOptions},
    session::{metrics::ConsoleMetrics, persistence::PersistenceManager},
};

#[derive(Debug, Clone)]
pub(super) struct LoadedMedia {
    media: MediaObject,
}

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub metrics: ConsoleMetrics,
    pub input_topology: Option<nerust_contract_input::InputTopologyDescriptor>,
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
}

impl SessionHandle {
    pub fn new_with_core(
        capabilities: HostBackendCapabilities,
        descriptor: SystemDescriptor,
        factory: Arc<dyn CoreFactory>,
    ) -> Self {
        use crate::settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        };
        let settings = SettingsManager::load_or_ephemeral(
            default_shared_settings(),
            default_local_settings(),
            default_app_state(),
        );
        let settings_snapshot = settings.snapshot().expect("settings snapshot should be readable");
        let (emu_core, input_adapter) = factory
            .create_core_and_adapter(&settings_snapshot)
            .expect("failed to create core");
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
        }
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
