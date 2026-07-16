pub mod access;
pub mod commands;
pub mod input;
pub mod lifecycle;
pub mod persistence;
#[cfg(test)]
pub(crate) mod test_helpers;
pub mod title;

use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use nerust_core_traits::{
    audio::AudioBackendRegistry,
    factory::{
        CoreFactory, FactoryError,
        descriptor::SystemSettingsPageModel,
        load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions},
    },
};
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, SettingsError, SettingsPaths, SettingsSnapshot,
    manager::SettingsManager,
};
use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_input_traits::{AttachmentId, DigitalControlId, GuiInput, InputAssignments};
use nerust_persistence::{error::PersistenceError, model::StateSlotSummary};
use nerust_render_base::{FrameBuffer, VideoRenderProfile};
use thiserror::Error;

use nerust_emu_thread::{ConsoleMetrics, OperationError};

use crate::{
    emu_core::EmuCore,
    session::persistence::PersistenceManager,
    settings::{self, factory::settings_view},
};

type CoreParts = (
    EmuCore,
    GuiInput,
    HashMap<(AttachmentId, DigitalControlId), usize>,
);

#[derive(Debug, Clone)]
pub(super) struct LoadedMedia {
    media: MediaObject,
}

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub metrics: ConsoleMetrics,
    pub slots: Arc<[StateSlotSummary]>,
    pub active_slot_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardShortcut {
    Session(ShortcutAction),
    ToggleFullscreen,
}

pub struct SessionHandle {
    pub(super) factory: Arc<dyn CoreFactory>,
    pub(super) emu_core: EmuCore,
    pub(super) gui_input: GuiInput,
    pub(super) current_assignments: InputAssignments,
    pub(super) field_map: HashMap<(AttachmentId, DigitalControlId), usize>,
    /// Reverse map: keyboard key → field index, rebuilt on binding/controller change.
    pub(super) key_field_map: HashMap<nerust_gui_settings::input::KeyboardKey, usize>,
    pub(super) capabilities: HostBackendCapabilities,
    pub(super) settings: SettingsManager,
    pub(super) settings_snapshot: SettingsSnapshot,
    pub(super) pressed_keys: BTreeSet<KeyboardKey>,
    pub(super) loaded_media: Option<LoadedMedia>,
    pub(super) persistence: PersistenceManager,
    pub(super) audio_registry: Arc<AudioBackendRegistry>,
}

impl SessionHandle {
    /// Load persisted controller assignments or fall back to defaults.
    fn load_assignments(
        factory: &Arc<dyn CoreFactory>,
        snapshot: &SettingsSnapshot,
        system_id: &str,
    ) -> InputAssignments {
        let persisted = snapshot.app_state.controller_assignments.get(system_id);
        match persisted {
            Some(pairs) => {
                let input_factory = factory.input_system_factory();
                let slots = pairs
                    .iter()
                    .filter_map(|(slot_id, ctrl_opt)| {
                        let att = match input_factory.resolve_slot(slot_id) {
                            Some(a) => a,
                            None => {
                                log::warn!("unknown persisted slot ID: {slot_id}");
                                return None;
                            }
                        };
                        let profile = ctrl_opt
                            .as_ref()
                            .and_then(|id| input_factory.resolve_controller(id));
                        Some((att, profile))
                    })
                    .collect();
                InputAssignments { slots }
            }
            None => factory.input_system_factory().default_assignments(),
        }
    }

    fn create_core_with_assignments(
        factory: &Arc<dyn CoreFactory>,
        registry: &AudioBackendRegistry,
        snapshot: &SettingsSnapshot,
        assignments: &InputAssignments,
    ) -> Result<CoreParts, SessionError> {
        let speaker = settings::build_speaker(registry, &snapshot.local);
        let system_id = factory.system_id();
        let view = settings_view(snapshot, &system_id);
        let parts =
            match factory.create_core_and_adapter_with_assignments(&view, speaker, assignments) {
                Ok(parts) => parts,
                Err(_) => {
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
                    let fallback_view = settings_view(&fallback, &system_id);
                    factory
                        .create_core_and_adapter(&fallback_view, fallback_speaker)
                        .map_err(|e| {
                            log::error!("core creation failed even with default settings: {e}");
                            SessionError::Factory(e)
                        })?
                }
            };
        Ok(EmuCore::from_parts(parts))
    }

    fn new_inner(
        capabilities: HostBackendCapabilities,
        factory: Arc<dyn CoreFactory>,
        audio_registry: Arc<AudioBackendRegistry>,
        use_persistent: bool,
    ) -> Result<Self, SessionError> {
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
        let sid = factory.system_id().to_string();
        let assignments = Self::load_assignments(&factory, &settings_snapshot, &sid);
        let (emu_core, gui_input, field_map) = Self::create_core_with_assignments(
            &factory,
            &audio_registry,
            &settings_snapshot,
            &assignments,
        )?;
        let mut result = Self {
            emu_core,
            gui_input,
            current_assignments: assignments,
            field_map,
            key_field_map: HashMap::new(),
            factory,
            capabilities,
            settings,
            settings_snapshot,
            pressed_keys: BTreeSet::new(),
            loaded_media: None,
            persistence: PersistenceManager::new(),
            audio_registry,
        };
        result.rebuild_key_field_map();
        Ok(result)
    }

    pub fn new(
        capabilities: HostBackendCapabilities,
        factory: Arc<dyn CoreFactory>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Result<Self, SessionError> {
        Self::new_inner(capabilities, factory, audio_registry, true)
    }

    /// Create a session with settings persisted at the given paths.
    ///
    /// On platforms where `ProjectDirs` is unavailable (e.g. Android),
    /// the frontend provides an explicit settings root instead.
    pub fn new_with_settings_paths(
        capabilities: HostBackendCapabilities,
        factory: Arc<dyn CoreFactory>,
        audio_registry: Arc<AudioBackendRegistry>,
        paths: SettingsPaths,
    ) -> Result<Self, SessionError> {
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
        let sid = factory.system_id().to_string();
        let assignments = Self::load_assignments(&factory, &settings_snapshot, &sid);
        let (emu_core, gui_input, field_map) = Self::create_core_with_assignments(
            &factory,
            &audio_registry,
            &settings_snapshot,
            &assignments,
        )?;
        let mut result = Self {
            emu_core,
            gui_input,
            current_assignments: assignments,
            field_map,
            key_field_map: HashMap::new(),
            factory,
            capabilities,
            settings,
            settings_snapshot,
            pressed_keys: BTreeSet::new(),
            loaded_media: None,
            persistence: PersistenceManager::new(),
            audio_registry,
        };
        result.rebuild_key_field_map();
        Ok(result)
    }

    #[cfg(test)]
    pub fn new_ephemeral(
        capabilities: HostBackendCapabilities,
        factory: Arc<dyn CoreFactory>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Self {
        Self::new_inner(capabilities, factory, audio_registry, false)
            .expect("core creation with defaults must succeed in tests")
    }

    /// Create a session or abort the process on catastrophic failure.
    /// The only scenario where this fails is when even default settings
    /// cannot produce a working core (e.g. missing GPU driver).
    pub fn new_or_abort(
        capabilities: HostBackendCapabilities,
        factory: Arc<dyn CoreFactory>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Self {
        Self::new(capabilities, factory, audio_registry).unwrap_or_else(|e| {
            log::error!("failed to create core: {e}");
            std::process::abort();
        })
    }

    /// Like `new_with_settings_paths`, but aborts on catastrophic failure.
    pub fn new_with_settings_paths_or_abort(
        capabilities: HostBackendCapabilities,
        factory: Arc<dyn CoreFactory>,
        audio_registry: Arc<AudioBackendRegistry>,
        paths: SettingsPaths,
    ) -> Self {
        Self::new_with_settings_paths(capabilities, factory, audio_registry, paths).unwrap_or_else(
            |e| {
                log::error!("failed to create core: {e}");
                std::process::abort();
            },
        )
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            metrics: self.emu_core.metrics(),
            slots: Arc::from(self.persistence.slots().to_vec()),
            active_slot_id: self.persistence.active_slot_id(),
        }
    }

    pub fn render_profile(&self) -> &VideoRenderProfile {
        self.emu_core.render_profile()
    }

    pub fn swap_frame_buffer(&mut self) {
        self.gui_input.publish();
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

    pub fn current_assignments_pairs(&self) -> Vec<(String, Option<String>)> {
        self.current_assignments.to_string_pairs()
    }

    pub fn settings_page(&self, settings: &SettingsSnapshot) -> SystemSettingsPageModel {
        let system_id = self.factory.system_id();
        let view = settings_view(settings, &system_id);
        self.factory.settings_page(&view)
    }

    pub fn default_load_options(&self) -> SystemLoadOptions {
        self.factory.default_load_options()
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

use crate::load::{RomLoadTarget, RomLoaderError};
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nerust_core_traits::{
        audio::AudioBackend,
        factory::{
            CoreFactory, FactoryError,
            load::{MediaObject, SystemLoadOptions},
            settings::FactorySettingsView,
        },
        identity::SystemId,
    };
    use nerust_gui_runtime::settings::SettingsApplyPlan;
    use nerust_input_traits::InputAssignments;

    use super::test_helpers::*;
    use crate::session::{KeyboardShortcut, SessionHandle};

    /// Factory that fails on first `create_core_and_adapter_with_assignments`
    /// call, then delegates to the inner factory for the fallback path.
    struct FailingOnceFactory {
        inner: Arc<dyn CoreFactory>,
        has_failed: std::sync::atomic::AtomicBool,
    }

    impl FailingOnceFactory {
        fn new(inner: Arc<dyn CoreFactory>) -> Self {
            Self {
                inner,
                has_failed: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl CoreFactory for FailingOnceFactory {
        fn system_id(&self) -> SystemId {
            self.inner.system_id()
        }
        fn display_name(&self) -> &'static str {
            self.inner.display_name()
        }
        fn create_core_and_adapter_with_assignments(
            &self,
            view: &FactorySettingsView,
            speaker: Box<dyn AudioBackend>,
            assignments: &InputAssignments,
        ) -> Result<nerust_core_traits::factory::CoreParts, FactoryError> {
            if !self
                .has_failed
                .swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                return Err(FactoryError::Create("simulated failure".into()));
            }
            self.inner
                .create_core_and_adapter_with_assignments(view, speaker, assignments)
        }
        fn create_core_and_adapter(
            &self,
            view: &FactorySettingsView,
            speaker: Box<dyn AudioBackend>,
        ) -> Result<nerust_core_traits::factory::CoreParts, FactoryError> {
            self.inner.create_core_and_adapter(view, speaker)
        }
        fn probe_media(&self, media: &nerust_core_traits::factory::load::MediaObject) -> bool {
            self.inner.probe_media(media)
        }
        fn settings_page(
            &self,
            view: &FactorySettingsView,
        ) -> nerust_core_traits::factory::descriptor::SystemSettingsPageModel {
            self.inner.settings_page(view)
        }
        fn apply_settings_choice(
            &self,
            view: &mut FactorySettingsView,
            field: &nerust_core_traits::factory::descriptor::SystemSettingsFieldId,
            choice: &nerust_core_traits::factory::descriptor::SystemSettingsChoiceId,
        ) -> Result<(), FactoryError> {
            self.inner.apply_settings_choice(view, field, choice)
        }
        fn resolve_load_request(
            &self,
            view: &FactorySettingsView,
            options: nerust_core_traits::factory::load::SystemLoadOptions,
        ) -> Result<nerust_core_traits::factory::load::ResolvedLoadRequest, FactoryError> {
            self.inner.resolve_load_request(view, options)
        }
        fn default_load_options(&self) -> nerust_core_traits::factory::load::SystemLoadOptions {
            self.inner.default_load_options()
        }
        fn input_system_factory(&self) -> &dyn nerust_input_traits::InputSystemFactory {
            self.inner.input_system_factory()
        }
    }

    #[test]
    fn shortcut_key_returns_shortcut_action_without_controller_event() {
        let mut session = test_session();
        assert_eq!(
            session.handle_keyboard_key(nerust_gui_settings::input::KeyboardKey::Space, true),
            Some(KeyboardShortcut::Session(
                nerust_gui_settings::input::ShortcutAction::TogglePause
            )),
        );
        assert_eq!(
            session.handle_keyboard_key(nerust_gui_settings::input::KeyboardKey::Space, true),
            None
        );
    }

    #[test]
    fn system_load_options_flow_into_session_load() {
        let mut session = test_session();
        let resolved = session
            .factory()
            .resolve_load_request(&test_view(&session), SystemLoadOptions::default())
            .unwrap();
        assert!(
            session
                .load_resolved(MediaObject::new(None, test_rom()), resolved)
                .is_ok()
        );
    }

    #[test]
    fn session_rebuild_reuses_previously_resolved_load_request() {
        let mut session = test_session();
        let options = session.factory().default_load_options();
        let resolved = session
            .factory()
            .resolve_load_request(&test_view(&session), options)
            .unwrap();
        session
            .load_resolved(MediaObject::new(None, test_rom()), resolved)
            .unwrap();
        assert!(session.loaded());

        let mut next = session.settings_snapshot().clone();
        next.local.audio.latency_ms = 90;
        let plan = session.apply_settings(next).unwrap();

        assert!(plan.session_rebuild_required);
        assert!(session.loaded());
    }

    #[test]
    fn set_fullscreen_default_updates_snapshot_and_plan() {
        let mut session = test_session();
        session.handle_keyboard_key(nerust_gui_settings::input::KeyboardKey::KeyZ, true);
        let plan = session
            .set_fullscreen_default(true)
            .expect("set_fullscreen_default should succeed");
        assert_eq!(
            plan,
            SettingsApplyPlan {
                window_settings_changed: true,
                fullscreen_default_changed: true,
                ..SettingsApplyPlan::default()
            }
        );
        assert!(
            session
                .settings_snapshot()
                .local
                .video
                .window
                .fullscreen_default
        );
        let second = session
            .set_fullscreen_default(true)
            .expect("second set_fullscreen_default should succeed");
        assert_eq!(second, SettingsApplyPlan::default());
    }

    #[test]
    fn session_creation_falls_back_to_defaults_when_custom_settings_fail() {
        let failing = Arc::new(FailingOnceFactory::new(Arc::new(MockFactory)));
        let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let capabilities = nerust_gui_runtime::settings::HostBackendCapabilities {
            window: nerust_gui_runtime::settings::HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        };
        let session = SessionHandle::new(capabilities, failing, audio_registry)
            .expect("fallback to defaults should succeed");
        assert!(!session.loaded());
        assert!(session.paused());
    }
}
