pub mod input;
mod lifecycle;
#[cfg(test)]
mod tests;

use crate::descriptor::{
    RuntimeHostServices, SystemDefinition, SystemDescriptor, SystemInputAdapter, SystemRuntime,
    default_system_definition,
};
use crate::load::{MediaObject, ResolvedLoadRequest};
use crate::settings::defaults::seed::{
    default_app_state, default_local_settings, default_shared_settings,
};
use nerust_console::ConsoleMetrics;
use nerust_console::video::{VideoFrameHandle, VideoRenderProfile};
use nerust_gui_runtime::settings::manager::SettingsManager;
use nerust_gui_runtime::settings::{HostBackendIdentity, SettingsSnapshot};
use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_persistence::model::StateSlotSummary;
use nerust_persistence::sidecar::SidecarPaths;
use nerust_screen_video::FrameBuffer;
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(super) struct LoadedMedia {
    media: MediaObject,
    request: ResolvedLoadRequest,
}

#[derive(Debug, Default)]
pub(super) struct PersistenceState {
    pub(super) sidecars: Option<SidecarPaths>,
    pub(super) mapper_save_flush_allowed: bool,
    pub(super) mapper_save_recovery_written: bool,
    pub(super) slots: Vec<StateSlotSummary>,
    pub(super) active_slot_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub system_id: Option<nerust_input_schema::SystemId>,
    pub metrics: ConsoleMetrics,
    pub input_topology: Option<nerust_input_schema::InputTopologyDescriptor>,
    pub video_frame: Option<VideoFrameHandle>,
    pub video_profile: Option<VideoRenderProfile>,
    pub slots: Arc<[StateSlotSummary]>,
    pub active_slot_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardShortcut {
    Session(ShortcutAction),
    ToggleFullscreen,
}

pub struct SessionHandle {
    pub(super) definition: Box<dyn SystemDefinition>,
    pub(super) descriptor: SystemDescriptor,
    pub(super) runtime: Box<dyn SystemRuntime>,
    pub(super) input_adapter: Box<dyn SystemInputAdapter>,
    pub(super) host_backend: HostBackendIdentity,
    pub(super) settings: SettingsManager,
    pub(super) settings_snapshot: SettingsSnapshot,
    pub(super) pressed_keys: BTreeSet<KeyboardKey>,
    pub(super) loaded_media: Option<LoadedMedia>,
    pub(super) persistence: PersistenceState,
}

impl Default for SessionHandle {
    fn default() -> Self {
        Self::new_for_host(HostBackendIdentity::gtk_opengl())
    }
}

impl SessionHandle {
    pub fn new_for_host(identity: HostBackendIdentity) -> Self {
        let settings = crate::settings::defaults::manager::load_settings_manager(identity);
        Self::new_with_settings_manager(identity, settings)
    }

    pub fn new_with_settings_manager(
        identity: HostBackendIdentity,
        settings: SettingsManager,
    ) -> Self {
        let settings_snapshot = crate::settings::defaults::manager::current_or_default(&settings);
        let definition = default_system_definition();
        let descriptor = definition.descriptor();
        let runtime = definition
            .create_runtime(
                &RuntimeHostServices {
                    host_backend: identity,
                },
                &settings_snapshot,
            )
            .expect("default runtime should build");
        Self {
            input_adapter: definition.create_input_adapter(&settings_snapshot),
            definition,
            descriptor,
            runtime,
            host_backend: identity,
            settings,
            settings_snapshot,
            pressed_keys: BTreeSet::new(),
            loaded_media: None,
            persistence: PersistenceState::default(),
        }
    }

    pub fn from_runtime(
        identity: HostBackendIdentity,
        runtime: Box<dyn SystemRuntime>,
        definition: Box<dyn SystemDefinition>,
    ) -> Self {
        let descriptor = definition.descriptor();
        let settings = SettingsManager::ephemeral(
            default_shared_settings(),
            default_local_settings(),
            default_app_state(),
        );
        let settings_snapshot = settings.snapshot().expect("ephemeral settings should read");
        Self {
            input_adapter: definition.create_input_adapter(&settings_snapshot),
            definition,
            descriptor,
            runtime,
            host_backend: identity,
            settings,
            settings_snapshot,
            pressed_keys: BTreeSet::new(),
            loaded_media: None,
            persistence: PersistenceState::default(),
        }
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        let runtime = self.runtime.snapshot();
        SessionSnapshot {
            system_id: Some(self.descriptor.system_id),
            metrics: runtime.metrics,
            input_topology: Some(self.descriptor.input_topology.clone()),
            video_frame: runtime.video_frame,
            video_profile: runtime.video_profile,
            slots: Arc::from(self.persistence.slots.clone()),
            active_slot_id: self.persistence.active_slot_id,
        }
    }

    pub fn swap_frame_buffer(&mut self) {
        self.runtime.swap_frame_buffer();
    }

    pub fn frame_buffer(&self) -> &FrameBuffer {
        self.runtime.frame_buffer()
    }

    pub fn settings_snapshot(&self) -> &SettingsSnapshot {
        &self.settings_snapshot
    }

    pub fn settings_manager(&self) -> &SettingsManager {
        &self.settings
    }

    pub fn with_frame_buffer(&self, f: &mut dyn FnMut(&[u8])) {
        self.runtime.with_frame_buffer(f);
    }
}
