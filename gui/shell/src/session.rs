pub mod access;
pub mod commands;
pub mod input;
pub mod lifecycle;
pub mod persistence;
#[cfg(test)]
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
        load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest},
    },
    identity::SystemId,
};
use nerust_emu_thread::{ConsoleMetrics, OperationError};
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, SettingsError, SettingsPaths, SettingsSnapshot,
    manager::SettingsManager,
};
use nerust_gui_settings::input::ShortcutAction;
use nerust_input_traits::{AttachmentId, DigitalControlId, GuiInput, InputAssignments};
use nerust_keyboard::Key;
use nerust_persistence::{error::PersistenceError, model::StateSlotSummary};
use nerust_render_traits::{FrameBuffer, VideoRenderProfile};
use thiserror::Error;

use crate::{
    emu_core::EmuCore,
    registry::SystemRegistry,
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
    pub(super) registry: Arc<SystemRegistry>,
    pub(super) active_system_id: SystemId,
    pub(super) emu_core: EmuCore,
    pub(super) gui_input: GuiInput,
    pub(super) current_assignments: InputAssignments,
    pub(super) field_map: HashMap<(AttachmentId, DigitalControlId), usize>,
    /// Reverse map: keyboard key → field index, rebuilt on binding/controller change.
    pub(super) key_field_map: HashMap<nerust_keyboard::Key, usize>,
    pub(super) capabilities: HostBackendCapabilities,
    pub(super) settings: SettingsManager,
    pub(super) settings_snapshot: SettingsSnapshot,
    pub(super) pressed_keys: BTreeSet<Key>,
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
        registry: Arc<SystemRegistry>,
        active_system_id: SystemId,
        audio_registry: Arc<AudioBackendRegistry>,
        use_persistent: bool,
    ) -> Result<Self, SessionError> {
        use crate::settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        };
        let factory = registry
            .find_by_id(&active_system_id)
            .unwrap_or_else(|| registry.primary())
            .clone();
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
            registry,
            active_system_id,
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
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Result<Self, SessionError> {
        let active_system_id = registry.primary().system_id();
        Self::new_inner(
            capabilities,
            registry,
            active_system_id,
            audio_registry,
            true,
        )
    }

    /// Create a session with settings persisted at the given paths.
    ///
    /// On platforms where `ProjectDirs` is unavailable (e.g. Android),
    /// the frontend provides an explicit settings root instead.
    pub fn new_with_settings_paths(
        capabilities: HostBackendCapabilities,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        paths: SettingsPaths,
    ) -> Result<Self, SessionError> {
        use crate::settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        };
        let factory = registry.primary().clone();
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
            registry,
            active_system_id: factory.system_id(),
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
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Self {
        let active_system_id = registry.primary().system_id();
        Self::new_inner(
            capabilities,
            registry,
            active_system_id,
            audio_registry,
            false,
        )
        .expect("core creation with defaults must succeed in tests")
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

    fn active_factory(&self) -> &Arc<dyn CoreFactory> {
        self.registry
            .find_by_id(&self.active_system_id)
            .unwrap_or_else(|| self.registry.primary())
    }

    pub fn active_factory_arc(&self) -> &Arc<dyn CoreFactory> {
        self.active_factory()
    }

    pub fn factory(&self) -> &dyn CoreFactory {
        &**self.active_factory()
    }

    pub fn current_assignments_pairs(&self) -> Vec<(String, Option<String>)> {
        self.current_assignments.to_string_pairs()
    }

    pub fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
        self.active_factory().default_load_options()
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

use crate::{
    load::{RomLoadTarget, RomLoaderError},
    session::commands::SessionCommand,
};

impl RomLoadTarget for SessionHandle {
    fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
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

    /// Notifies the session that a ROM from a different system has been loaded.
    ///
    /// Updates `active_system_id` so that subsequent `factory()`,
    /// `default_load_options()`, and settings page queries use the
    /// correct system's factory.
    ///
    /// Note: when multi-system support is added, this method must also
    /// trigger an EmuCore rebuild so that the running core matches the
    /// detected system. Currently only NES cores exist, so the existing
    /// EmuCore is always correct.
    fn set_active_system(&mut self, system_id: SystemId) {
        if system_id == self.active_system_id {
            return;
        }
        self.active_system_id = system_id;

        // Rebuild EmuCore with the new system's factory.
        // If a ROM is currently loaded, its state is preserved
        // and restored into the new core.
        let snapshot = self.settings_snapshot.clone();
        if let Err(e) = self.rebuild_for_settings(&snapshot) {
            log::error!("failed to rebuild core for {}: {e}", system_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::Ordering::AcqRel};

    use nerust_core_traits::{
        audio::AudioBackend,
        factory::{
            CoreFactory, CoreParts, FactoryError,
            descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel},
            load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest},
            settings::FactorySettingsView,
        },
        identity::SystemId,
    };
    use nerust_gui_runtime::settings::{
        HostBackendCapabilities, SettingsApplyPlan, SettingsSnapshot,
    };
    use nerust_input_traits::{InputAssignments, InputSystemFactory};

    use super::test_helpers::*;
    use crate::{
        load::RomLoadTarget,
        registry::SystemRegistry,
        session::{KeyboardShortcut, SessionHandle},
        settings::factory::settings_view,
    };

    /// Factory that fails on first `create_core_and_adapter_with_assignments`
    /// call, then delegates to the inner factory for the fallback path.
    struct FailingOnceFactory<T: CoreFactory> {
        inner: Arc<T>,
        has_failed: std::sync::atomic::AtomicBool,
    }

    impl<T: CoreFactory> FailingOnceFactory<T> {
        fn new(inner: Arc<T>) -> Self {
            Self {
                inner,
                has_failed: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl<T: CoreFactory> CoreFactory for FailingOnceFactory<T> {
        fn system_id(&self) -> SystemId {
            self.inner.system_id()
        }
        fn display_name(&self) -> &'static str {
            self.inner.display_name()
        }
        fn create_core_and_adapter_with_assignments(
            &self,
            _: &FactorySettingsView,
            _: Box<dyn AudioBackend>,
            _: &InputAssignments,
        ) -> Result<CoreParts, FactoryError> {
            if self.has_failed.swap(true, AcqRel) {
                unreachable!()
            }
            Err(FactoryError::Create("simulated failure".into()))
        }
        fn create_core_and_adapter(
            &self,
            view: &FactorySettingsView,
            speaker: Box<dyn AudioBackend>,
        ) -> Result<CoreParts, FactoryError> {
            self.inner.create_core_and_adapter(view, speaker)
        }
        fn probe_media(&self, _media: &MediaObject) -> bool {
            unreachable!()
        }
        fn settings_page(&self, _: &FactorySettingsView) -> SystemSettingsPageModel {
            unreachable!()
        }
        fn apply_settings_choice(
            &self,
            _: &mut FactorySettingsView,
            _: &SystemSettingsFieldId,
            _: &SystemSettingsChoiceId,
        ) -> Result<(), FactoryError> {
            unreachable!()
        }
        fn resolve_load_request(
            &self,
            _: &FactorySettingsView,
            _: Box<dyn DynSystemLoadOptions>,
        ) -> Result<ResolvedLoadRequest, FactoryError> {
            unreachable!()
        }
        fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
            unreachable!()
        }
        fn input_system_factory(&self) -> &dyn InputSystemFactory {
            self.inner.input_system_factory()
        }

        fn load_options_schema(
            &self,
        ) -> Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptionsSchema> {
            // CLI parsing not exercised in this test path
            unreachable!()
        }
    }

    #[test]
    fn shortcut_key_returns_shortcut_action_without_controller_event() {
        let mut session = test_session();
        assert_eq!(
            session.handle_keyboard_key(nerust_keyboard::Key::Space, true),
            Some(KeyboardShortcut::Session(
                nerust_gui_settings::input::ShortcutAction::TogglePause
            )),
        );
        assert_eq!(
            session.handle_keyboard_key(nerust_keyboard::Key::Space, true),
            None
        );
    }

    #[test]
    fn system_load_options_flow_into_session_load() {
        let mut session = test_session();
        let resolved = session
            .factory()
            .resolve_load_request(
                &test_view(&session),
                session.factory().default_load_options(),
            )
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
        session.handle_keyboard_key(nerust_keyboard::Key::KeyZ, true);
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
        let registry = Arc::new(SystemRegistry::new(vec![failing]));
        let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let capabilities = nerust_gui_runtime::settings::HostBackendCapabilities {
            window: nerust_gui_runtime::settings::HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        };
        let session = SessionHandle::new(capabilities, registry, audio_registry)
            .expect("fallback to defaults should succeed");
        assert!(!session.loaded());
        assert!(session.paused());
    }

    #[test]
    fn session_factory_uses_primary_initially() {
        let factory = Arc::new(MockFactory);
        let id = factory.system_id();
        let registry = Arc::new(SystemRegistry::new(vec![factory]));
        let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
        assert_eq!(session.factory().system_id(), id);
    }

    #[test]
    fn set_active_system_falls_back_to_primary_on_unknown_id() {
        let factory = Arc::new(MockFactory);
        let id = factory.system_id();
        let registry = Arc::new(SystemRegistry::new(vec![factory]));
        let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let mut session =
            SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);

        RomLoadTarget::set_active_system(&mut session, SystemId::new("unknown"));
        assert_eq!(session.factory().system_id(), id);
    }

    fn test_capabilities() -> HostBackendCapabilities {
        HostBackendCapabilities {
            window: nerust_gui_runtime::settings::HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: false,
                supports_scaling: false,
            },
            presentation: None,
        }
    }

    #[test]
    fn registry_all_produces_settings_page_per_system() {
        use crate::settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        };
        let factory = Arc::new(MockFactory);
        let registry = SystemRegistry::new(vec![factory.clone(), factory]);
        let snapshot = SettingsSnapshot {
            shared: default_shared_settings(),
            local: default_local_settings(),
            app_state: default_app_state(),
        };
        let pages: Vec<_> = registry
            .all()
            .iter()
            .map(|f| {
                let view = settings_view(&snapshot, &f.system_id());
                f.settings_page(&view)
            })
            .collect();
        assert_eq!(
            pages.len(),
            2,
            "should produce one page per registered system"
        );
    }
}
