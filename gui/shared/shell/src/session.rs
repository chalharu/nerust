pub mod commands;
pub mod input;
mod lifecycle;
pub mod metrics;
#[cfg(test)]
mod tests;
pub mod title;

pub use lifecycle::WindowSize;

use crate::descriptor::{SystemDescriptor, SystemInputAdapter, default_system_descriptor};
use crate::emu_core::EmuCore;
use crate::load::{MediaObject, ResolvedLoadRequest};
use crate::session::metrics::ConsoleMetrics;
use nerust_gui_runtime::settings::manager::SettingsManager;
use nerust_gui_runtime::settings::{HostBackendIdentity, SettingsSnapshot};
use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_persistence::model::StateSlotSummary;
use nerust_persistence::sidecar::SidecarPaths;
use nerust_screen_video::FrameBuffer;
use nerust_screen_video::{VideoFrameHandle, VideoRenderProfile};
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
    pub(super) emu_core: EmuCore,
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
        let descriptor = default_system_descriptor();
        let (emu_core, input_adapter) =
            crate::descriptor::create_core_and_adapter(&settings_snapshot)
                .expect("default core should build");
        Self {
            emu_core,
            input_adapter,
            descriptor,
            host_backend: identity,
            settings,
            settings_snapshot,
            pressed_keys: BTreeSet::new(),
            loaded_media: None,
            persistence: PersistenceState::default(),
        }
    }

    #[cfg(test)]
    pub fn new_with_core(
        identity: HostBackendIdentity,
        emu_core: EmuCore,
        input_adapter: Box<dyn SystemInputAdapter>,
    ) -> Self {
        use crate::settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        };
        let descriptor = default_system_descriptor();
        let settings = SettingsManager::ephemeral(
            default_shared_settings(),
            default_local_settings(),
            default_app_state(),
        );
        let settings_snapshot = settings.snapshot().expect("ephemeral settings should read");
        Self {
            emu_core,
            input_adapter,
            descriptor,
            host_backend: identity,
            settings,
            settings_snapshot,
            pressed_keys: BTreeSet::new(),
            loaded_media: None,
            persistence: PersistenceState::default(),
        }
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        let core_snapshot = self.emu_core.snapshot();
        SessionSnapshot {
            system_id: Some(self.descriptor.system_id),
            metrics: core_snapshot.metrics,
            input_topology: Some(self.descriptor.input_topology.clone()),
            video_frame: core_snapshot.video_frame,
            slots: Arc::from(self.persistence.slots.clone()),
            active_slot_id: self.persistence.active_slot_id,
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

    pub fn settings_snapshot(&self) -> &SettingsSnapshot {
        &self.settings_snapshot
    }

    pub fn settings_manager(&self) -> &SettingsManager {
        &self.settings
    }

    pub fn with_frame_buffer(&self, f: &mut dyn FnMut(&[u8])) {
        f(self.emu_core.frame_buffer().as_ref());
    }
}
