pub mod access;
pub(crate) mod command_exec;
pub mod commands;
pub mod debug_ops;
pub(crate) mod debug_snapshot;
pub mod error;
pub mod fs;
pub mod input;
pub(crate) mod input_map;
pub mod lifecycle;
pub mod lifecycle_persist;
pub mod load_adapter;
pub mod notification;
pub mod osd;
pub mod osd_history;
pub(crate) mod osd_modal;
pub(crate) mod osd_presenter;
pub(crate) mod osd_render;
pub(crate) mod osd_transient;
pub mod persistence;
pub(crate) mod persistence_setup;
#[cfg(test)]
mod persistence_test;
pub mod ports;
pub mod services;
pub mod session_query;
pub(crate) mod settings_apply;
pub mod title;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use nerust_core_traits::{
    audio::AudioBackendRegistry,
    factory::{
        CoreFactory, FactoryError,
        load::{DynSystemLoadOptions, MediaObject},
    },
    identity::SystemId,
    touch::TouchOverlayModel,
};
use nerust_emu_thread::OperationError;
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, SettingsError, SettingsPaths, SettingsSnapshot,
    manager::SettingsManager,
};
use nerust_gui_viewmodel::debugger as debug_vm;
use nerust_input_traits::{AttachmentId, DigitalControlId, InputAssignments};
use nerust_keyboard::Key;
use nerust_persistence::error::PersistenceError;
use nerust_render_traits::{FrameBuffer, VideoRenderProfile};
use nerust_settings_core::factory::settings_view;
use nerust_settings_core::model::input::ShortcutAction;
use thiserror::Error;

use crate::{
    emu_core::{EmuCore, LiveCore},
    registry::SystemRegistry,
    session::persistence::{PersistenceBackends, PersistenceManager, StateExporter},
};

/// Live PPU image batch; canonical home is [`emu_core::images`](crate::emu_core::images).
pub use crate::emu_core::images::LiveImages;

struct CoreRuntime {
    emu_core: EmuCore,
    gui_input: Box<dyn nerust_core_traits::factory::GuiInputPort>,
    field_map: HashMap<(AttachmentId, DigitalControlId), usize>,
    host_peripherals: nerust_core_traits::peripheral::HostPeripheralHandles,
}

impl CoreRuntime {
    fn from_factory_parts(parts: nerust_core_traits::factory::CoreParts) -> Self {
        let (emu_core, gui_input, field_map, host_peripherals) = EmuCore::from_parts(parts);
        Self {
            emu_core,
            gui_input,
            field_map,
            host_peripherals,
        }
    }
}

struct CoreCreation {
    runtime: CoreRuntime,
    applied_assignments: InputAssignments,
}

#[derive(Debug, Clone)]
pub(super) struct LoadedMedia {
    media: MediaObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardShortcut {
    Session(ShortcutAction),
    ToggleFullscreen,
    /// Open the debugger window, not a session command.
    /// Frontends route it to their debugger-open path.
    OpenDebugger,
}

/// Input department: assignments, key maps, live runtime. One owner
/// for the input domain so `SessionHandle` stays a facade.
struct InputDept {
    current_assignments: InputAssignments,
    field_map: HashMap<(AttachmentId, DigitalControlId), usize>,
    host_peripherals: nerust_core_traits::peripheral::HostPeripheralHandles,
    /// Reverse map: keyboard key → field index, rebuilt on binding/controller change.
    key_field_map: HashMap<nerust_keyboard::Key, usize>,
    pressed_keys: std::collections::HashSet<Key>,
    /// Last served touch overlay, keyed by the caller revision.
    /// A repeat revision always means identical controls.
    touch_cache: Option<crate::session::input::TouchCache>,
    /// Empty overlay served if the cache write is ever lost. Keeps the
    /// cached accessor infallible without an `expect` on the hot path.
    fallback_model: TouchOverlayModel,
}

impl InputDept {
    /// Rebuild the keyboard reverse map from the implicit profile.
    /// Takes only the bindings it reads; never reaches into settings or the registry.
    pub(crate) fn rebuild_key_field_map(
        &mut self,
        bindings: Option<&[nerust_settings_core::model::input::KeyboardBinding]>,
    ) {
        self.key_field_map.clear();
        if let Some(bindings) = bindings
            && let Err(duplicate) = crate::session::input_map::rebuild_input_map(
                &self.field_map,
                bindings,
                &mut self.key_field_map,
            )
        {
            log::warn!(
                "duplicate input binding for {attachment}/{control}; keeping first",
                attachment = duplicate.attachment,
                control = duplicate.control,
            );
        }
    }

    /// Persistable projection of the live assignments: only slot and controller ids survive.
    /// An explicitly named projection, not a `From` conversion.
    fn persistable_pairs(&self) -> Vec<(String, Option<String>)> {
        self.current_assignments
            .slots()
            .iter()
            .map(|(slot, controller)| {
                (
                    slot.as_str().to_owned(),
                    controller
                        .as_ref()
                        .map(|p| p.profile_id().as_str().to_owned()),
                )
            })
            .collect()
    }
}

/// Settings cluster: manager plus its snapshot. Single owner for the
/// settings domain.
struct SettingsCluster {
    manager: SettingsManager,
    snapshot: SettingsSnapshot,
}

/// Media department: loaded media plus persistence. Single owner
/// for the save/load domain.
struct MediaDept {
    loaded: Option<LoadedMedia>,
    persistence: PersistenceManager,
}

impl MediaDept {
    fn set_loaded(&mut self, loaded: Option<LoadedMedia>) {
        self.loaded = loaded;
    }

    fn clear_runtime(&mut self) {
        self.loaded = None;
        self.persistence.reset();
    }
}

/// Live emulation objects: the core plus its GUI input adapter.
/// Everything touching the running core goes through these typed methods.
struct EmuCluster {
    core: Option<EmuCore>,
    input: Option<Box<dyn nerust_core_traits::factory::GuiInputPort>>,
}

impl EmuCluster {
    fn new() -> Self {
        Self {
            core: None,
            input: None,
        }
    }

    /// Single typed access: shared live core.
    pub(crate) fn require_live(&self) -> Result<&EmuCore, SessionError> {
        self.core.as_ref().ok_or(SessionError::NoCore)
    }

    /// Single typed access: exclusive live core.
    pub(crate) fn require_live_mut(&mut self) -> Result<&mut EmuCore, SessionError> {
        self.core.as_mut().ok_or(SessionError::NoCore)
    }

    /// Shared live core behind the [`LiveCore`](crate::emu_core::LiveCore) port.
    pub(crate) fn core(&self) -> Result<&dyn LiveCore, SessionError> {
        self.require_live().map(|core| core as &dyn LiveCore)
    }

    /// Exclusive live core for swaps and rebuilds.
    pub(crate) fn core_mut(&mut self) -> Result<&mut dyn LiveCore, SessionError> {
        self.require_live_mut()
            .map(|core| core as &mut dyn LiveCore)
    }

    /// Persistence port of the live core.
    pub(crate) fn exporter(&self) -> Result<&dyn StateExporter, SessionError> {
        self.require_live().map(|core| core as &dyn StateExporter)
    }

    /// Paired install: core and input always arrive together from the
    /// factory, so they are never half-present.
    fn install(
        &mut self,
        core: EmuCore,
        input: Box<dyn nerust_core_traits::factory::GuiInputPort>,
    ) {
        self.core = Some(core);
        self.input = Some(input);
    }

    fn input_mut(
        &mut self,
    ) -> Option<&mut (dyn nerust_core_traits::factory::GuiInputPort + 'static)> {
        self.input.as_mut().map(|boxed| boxed.as_mut())
    }
}

pub struct SessionHandle {
    registry: Arc<SystemRegistry>,
    active_system_id: Option<Box<dyn SystemId>>,
    input: InputDept,
    emu: EmuCluster,
    capabilities: HostBackendCapabilities,
    settings: SettingsCluster,
    media: MediaDept,
    audio_registry: Arc<AudioBackendRegistry>,
    /// Shell-owned two-phase memory write transaction (Phase B).
    /// `Arc`: async debug-drain flights share it across UI ticks.
    debug_txn: Arc<Mutex<debug_vm::WriteTransaction>>,
    notifications: notification::NotificationCenter,
    title_cache: title::TitleCache,
    history_view: session_query::HistoryView,
}

impl SessionHandle {
    /// Load persisted controller assignments or fall back to defaults.
    fn load_assignments(
        factory: &Arc<dyn CoreFactory>,
        snapshot: &SettingsSnapshot,
        system_id: &dyn SystemId,
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
                InputAssignments::new(slots)
            }
            None => factory.input_system_factory().default_assignments(),
        }
    }

    fn create_core_with_assignments(
        factory: &Arc<dyn CoreFactory>,
        speaker_factory: &dyn crate::settings::AudioSpeakerFactory,
        snapshot: &SettingsSnapshot,
        assignments: &InputAssignments,
    ) -> Result<CoreCreation, SessionError> {
        let speaker = speaker_factory.build_speaker(&snapshot.local);
        let system_id = factory.system_id();
        let view = settings_view(snapshot, system_id.as_ref());
        let (parts, applied_assignments) =
            match factory.create_core_and_adapter_with_assignments(&view, speaker, assignments) {
                Ok(parts) => (parts, assignments.clone()),
                Err(_) => {
                    log::warn!("core creation with loaded settings failed; using defaults");
                    use crate::settings::defaults::seed::{
                        default_app_state, default_local_settings, default_shared_settings,
                    };
                    let fallback = SettingsSnapshot {
                        shared: default_shared_settings(std::slice::from_ref(factory)),
                        local: default_local_settings(),
                        app_state: default_app_state(),
                    };
                    let fallback_speaker = speaker_factory.build_speaker(&fallback.local);
                    let fallback_view = settings_view(&fallback, system_id.as_ref());
                    let fallback_assignments = factory.input_system_factory().default_assignments();
                    let parts = factory
                        .create_core_and_adapter_with_assignments(
                            &fallback_view,
                            fallback_speaker,
                            &fallback_assignments,
                        )
                        .map_err(|e| {
                            log::error!("core creation failed even with default settings: {e}");
                            SessionError::Factory(e)
                        })?;
                    (parts, fallback_assignments)
                }
            };
        Ok(CoreCreation {
            runtime: CoreRuntime::from_factory_parts(parts),
            applied_assignments,
        })
    }

    fn init_settings_manager(
        registry: &SystemRegistry,
        use_persistent: bool,
        paths: Option<SettingsPaths>,
    ) -> (SettingsManager, SettingsSnapshot) {
        use crate::settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        };
        let defaults_shared = default_shared_settings(registry.all());
        let settings = if let Some(paths) = paths {
            SettingsManager::load_or_ephemeral_with_paths(
                paths,
                defaults_shared,
                default_local_settings(),
                default_app_state(),
            )
        } else if use_persistent {
            SettingsManager::load_or_ephemeral(
                defaults_shared,
                default_local_settings(),
                default_app_state(),
            )
        } else {
            SettingsManager::ephemeral(
                defaults_shared,
                default_local_settings(),
                default_app_state(),
            )
        };
        let settings_snapshot = settings.snapshot().unwrap_or_else(|e| {
            log::warn!("settings snapshot unavailable, using ephemeral defaults: {e}");
            SettingsSnapshot {
                shared: default_shared_settings(registry.all()),
                local: default_local_settings(),
                app_state: default_app_state(),
            }
        });
        (settings, settings_snapshot)
    }

    fn new_inner(
        capabilities: HostBackendCapabilities,
        registry: Arc<SystemRegistry>,
        active_system_id: Option<Box<dyn SystemId>>,
        audio_registry: Arc<AudioBackendRegistry>,
        use_persistent: bool,
    ) -> Result<Self, SessionError> {
        Self::new_inner_with_paths(
            capabilities,
            registry,
            active_system_id,
            audio_registry,
            use_persistent,
            None,
        )
    }

    fn new_inner_with_paths(
        capabilities: HostBackendCapabilities,
        registry: Arc<SystemRegistry>,
        active_system_id: Option<Box<dyn SystemId>>,
        audio_registry: Arc<AudioBackendRegistry>,
        use_persistent: bool,
        paths: Option<SettingsPaths>,
    ) -> Result<Self, SessionError> {
        let (settings, settings_snapshot) =
            Self::init_settings_manager(&registry, use_persistent, paths);
        let factory = active_system_id
            .as_ref()
            .and_then(|id| registry.find_by_id(id.as_ref()))
            .cloned();
        let (emu_core, gui_input, field_map, host_peripherals, assignments) = if let Some(ref f) =
            factory
        {
            let sid = f.system_id();
            let requested_assignments = Self::load_assignments(f, &settings_snapshot, sid.as_ref());
            let created = Self::create_core_with_assignments(
                f,
                audio_registry.as_ref(),
                &settings_snapshot,
                &requested_assignments,
            )?;
            (
                Some(created.runtime.emu_core),
                Some(created.runtime.gui_input),
                created.runtime.field_map,
                created.runtime.host_peripherals,
                created.applied_assignments,
            )
        } else {
            (
                None,
                None,
                HashMap::new(),
                Default::default(),
                InputAssignments::new(vec![]),
            )
        };
        let mut result = Self {
            input: InputDept {
                current_assignments: assignments,
                field_map,
                host_peripherals,
                key_field_map: HashMap::new(),
                pressed_keys: std::collections::HashSet::new(),
                touch_cache: None,
                fallback_model: TouchOverlayModel {
                    revision: 0,
                    controls: Vec::new(),
                },
            },
            emu: EmuCluster::new(),
            registry,
            active_system_id,
            capabilities,
            settings: SettingsCluster {
                manager: settings,
                snapshot: settings_snapshot,
            },
            media: MediaDept {
                loaded: None,
                persistence: PersistenceManager::new(),
            },
            audio_registry,
            debug_txn: Arc::new(Mutex::new(debug_vm::WriteTransaction::Empty)),
            notifications: notification::NotificationCenter::new(),
            title_cache: title::TitleCache::new(),
            history_view: session_query::HistoryView::new(),
        };
        if let (Some(core), Some(input)) = (emu_core, gui_input) {
            result.emu.install(core, input);
        }
        result.rebuild_key_field_map();
        Ok(result)
    }

    pub fn new(
        capabilities: HostBackendCapabilities,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Result<Self, SessionError> {
        Self::new_inner(capabilities, registry, None, audio_registry, true)
    }

    /// Create a session with settings persisted at the given paths.
    /// Where `ProjectDirs` is unavailable, the frontend provides an explicit settings root.
    pub fn new_with_settings_paths(
        capabilities: HostBackendCapabilities,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        paths: SettingsPaths,
    ) -> Result<Self, SessionError> {
        Self::new_inner_with_paths(
            capabilities,
            registry,
            None,
            audio_registry,
            true,
            Some(paths),
        )
    }

    #[cfg(test)]
    pub fn new_ephemeral(
        capabilities: HostBackendCapabilities,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Self {
        Self::new_inner(capabilities, registry, None, audio_registry, false)
            .expect("core creation with defaults must succeed in tests")
    }

    pub fn render_profile(&self) -> Option<&VideoRenderProfile> {
        self.emu.core().ok().map(|c| c.render_profile())
    }

    /// Swap the display buffer, then report the fresh tick.
    /// One metrics lock for the whole pass.
    pub fn swap_frame_buffer_tick(&mut self) -> osd::TickMetrics {
        self.swap_frame_buffer();
        self.tick_metrics()
    }

    pub fn swap_frame_buffer(&mut self) {
        if let Some(gui_input) = self.emu.input_mut() {
            gui_input.publish();
        }
        if let Ok(core) = self.emu.core_mut() {
            core.swap_frame_buffer();
        }
    }

    /// Borrow the display buffer for one render call without leaking the render type.
    /// Prefer this over [`frame_buffer`](Self::frame_buffer).
    pub fn with_frame_buffer<R>(&self, render: impl FnOnce(&FrameBuffer) -> R) -> Option<R> {
        self.emu.core().ok().map(|c| render(c.frame_buffer()))
    }

    /// Infra escape hatch: raw display buffer.
    /// New code prefers [`with_frame_buffer`](Self::with_frame_buffer); kept as a shim.
    pub fn frame_buffer(&self) -> Option<&FrameBuffer> {
        self.emu.core().ok().map(|c| c.frame_buffer())
    }

    pub fn clear_display(&mut self) {
        if let Ok(core) = self.emu.core_mut() {
            core.clear_display();
        }
    }

    pub fn settings_snapshot(&self) -> &SettingsSnapshot {
        &self.settings.snapshot
    }

    /// Narrow chrome view for the OSD layer decision, adapted once at the settings boundary.
    pub fn chrome(&self) -> osd::OsdChrome {
        osd::OsdChrome::from(&self.settings.snapshot.local.notifications)
    }

    /// Narrow transient policy for OSD pushes, adapted once at the
    /// settings boundary.
    pub fn transient_policy(&self) -> osd::TransientPolicy {
        osd::TransientPolicy::from(&self.settings.snapshot.local.notifications)
    }

    /// Configured save locations, if persistence setup resolved them.
    /// Narrow sidecar view: cloned paths, no manager access needed.
    pub fn sidecars(&self) -> Option<(PathBuf, PathBuf)> {
        self.media.persistence.configured_sidecars()
    }

    pub fn host_peripherals(&self) -> &nerust_core_traits::peripheral::HostPeripheralHandles {
        &self.input.host_peripherals
    }

    pub fn current_assignments(&self) -> &InputAssignments {
        &self.input.current_assignments
    }

    #[cfg(test)]
    pub(crate) fn set_test_assignments(&mut self, assignments: InputAssignments) {
        self.input.current_assignments = assignments;
    }

    #[cfg(test)]
    pub(crate) fn loaded_media_ref(&self) -> &Option<LoadedMedia> {
        &self.media.loaded
    }

    /// Infra escape hatch: full settings manager; new code uses narrow readers and writers.
    /// Kept as a shim: removing it breaks out-of-crate frontends.
    pub fn settings_manager(&self) -> &SettingsManager {
        &self.settings.manager
    }

    /// Bundle form of the backend swap: one struct instead of three
    /// positional boxes. Test and Android backends enter here.
    pub fn configure_persistence_backends(&mut self, backends: PersistenceBackends) {
        self.media.persistence.configure_backends(backends);
    }

    /// Infra escape hatch: three positional backend boxes.
    /// New code prefers [`configure_persistence_backends`](Self::configure_persistence_backends).
    pub fn set_persistence_backends(
        &mut self,
        slot_backend: Box<dyn persistence::SlotBackend>,
        autosave_backend: Box<dyn persistence::AutoSaveBackend>,
        mapper_backend: Box<dyn persistence::MapperSaveBackend>,
    ) {
        self.configure_persistence_backends(PersistenceBackends {
            slot: slot_backend,
            autosave: autosave_backend,
            mapper: mapper_backend,
        });
    }

    #[cfg(test)]
    pub(crate) fn set_settings(&mut self, settings: SettingsManager) {
        self.settings.manager = settings;
    }

    #[cfg(test)]
    pub(crate) fn set_settings_snapshot(&mut self, snapshot: SettingsSnapshot) {
        self.settings.snapshot = snapshot;
    }

    /// Port for building live speakers: session code depends on this seam, never the registry.
    pub(crate) fn speaker_factory(&self) -> &dyn crate::settings::AudioSpeakerFactory {
        self.audio_registry.as_ref()
    }

    /// Narrow active-factory query: clone the factory without exposing
    /// the registry lookup. Same-crate settings/input paths use this.
    pub(crate) fn active_factory_cloned(&self) -> Option<Arc<dyn CoreFactory>> {
        self.active_system_id
            .as_ref()
            .and_then(|id| self.registry.find_by_id(id.as_ref()))
            .cloned()
    }

    /// Narrow active-factory query: run one closure against the live factory.
    /// Same-crate orchestration uses this instead of holding the factory across borrows.
    pub(crate) fn with_active_factory<R>(
        &self,
        use_factory: impl FnOnce(&dyn CoreFactory) -> R,
    ) -> Option<R> {
        self.active_system_id
            .as_ref()
            .and_then(|id| self.registry.find_by_id(id.as_ref()))
            .map(|factory| use_factory(factory.as_ref()))
    }

    /// Narrow active-system query: display name for titles, without
    /// exposing the factory or the registry.
    pub fn active_system_display_name(&self) -> Option<&'static str> {
        self.with_active_factory(|factory| factory.display_name())
    }

    /// Narrow active-system query: owned id for settings lookups.
    pub(crate) fn active_system_id_box(&self) -> Option<Box<dyn SystemId>> {
        self.active_system_id.as_ref().map(|id| id.clone_box())
    }

    /// Infra escape hatch: live factory behind the registry.
    /// Same-crate code uses [`active_factory_cloned`](Self::active_factory_cloned) instead.
    pub fn active_factory(&self) -> Option<&Arc<dyn CoreFactory>> {
        self.active_system_id
            .as_ref()
            .and_then(|id| self.registry.find_by_id(id.as_ref()))
    }

    /// Infra escape hatch: full system registry; new code uses narrow `active_system_*` queries.
    pub fn registry(&self) -> &SystemRegistry {
        &self.registry
    }

    pub fn factory(&self) -> Option<&dyn CoreFactory> {
        self.active_system_id
            .as_ref()
            .and_then(|id| self.registry.find_by_id(id.as_ref()))
            .map(|a| &**a)
    }

    pub fn active_system_id(&self) -> Option<&dyn SystemId> {
        self.active_system_id.as_deref()
    }

    /// Persistable projection of the live assignments: only slot and controller ids survive.
    /// An explicitly named projection, not a `From` conversion.
    pub fn current_assignments_pairs(&self) -> Vec<(String, Option<String>)> {
        self.input.persistable_pairs()
    }

    pub fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
        self.with_active_factory(|f| f.default_load_options())
    }

    /// Cached window title for the current tick; rebuilds only on tick, selection, or name change.
    pub fn window_title_cached(&mut self, tick: &osd::TickMetrics) -> &str {
        let name = self.active_system_display_name();
        let input = title::TitleInput {
            paused: tick.paused,
            loaded: tick.loaded,
            system_name: name,
            active_slot: self.active_slot_id(),
            slot_count: self.slots().len(),
        };
        self.title_cache.title(input)
    }

    /// Cached history rows; rebuilds only on history advance or language change.
    pub fn notification_history_lines_cached(
        &mut self,
        language: nerust_settings_core::model::language::AppLanguage,
    ) -> &[osd::HistoryLine] {
        self.history_view.lines(
            self.notifications.history(),
            self.notifications.history_seq(),
            language,
        )
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
    #[error("no emulation core active")]
    NoCore,
}

use crate::load::SystemActivationError;

impl SessionHandle {
    /// Records the detected system before ROM load so the core is rebuilt with the right factory.
    /// Entry point for lazy activation; the load adapter delegates here.
    pub fn set_active_system(
        &mut self,
        system_id: &dyn SystemId,
    ) -> Result<(), SystemActivationError> {
        if self.active_system_id.as_ref().map(|x| x.as_ref()) == Some(system_id) {
            return Ok(());
        }
        let factory = self
            .registry
            .find_by_id(system_id)
            .cloned()
            .ok_or(SystemActivationError::NotRegistered(system_id.clone_box()))?;
        let requested_assignments =
            Self::load_assignments(&factory, &self.settings.snapshot, system_id);
        let created = Self::create_core_with_assignments(
            &factory,
            self.speaker_factory(),
            &self.settings.snapshot,
            &requested_assignments,
        )
        .map_err(SystemActivationError::from_session)?;

        if let Ok(exporter) = self.emu.exporter() {
            self.media
                .persistence
                .flush_mapper_save(exporter)
                .map_err(|e| SystemActivationError::Activation(e.to_string()))?;
        }

        self.active_system_id = Some(system_id.clone_box());
        self.input.current_assignments = created.applied_assignments;
        self.emu
            .install(created.runtime.emu_core, created.runtime.gui_input);
        self.input.field_map = created.runtime.field_map;
        self.input.host_peripherals = created.runtime.host_peripherals;
        self.media.clear_runtime();
        self.input.pressed_keys.clear();
        self.rebuild_key_field_map();
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use std::sync::Arc;

    use nerust_core_traits::{
        audio::AudioBackendRegistry,
        factory::{CoreFactory, settings::FactorySettingsView},
    };
    use nerust_gui_runtime::settings::{HostBackendCapabilities, HostWindowCapabilities};

    use crate::test_helpers::MockFactory;
    use crate::{registry::SystemRegistry, session::SessionHandle};

    pub(crate) fn test_session() -> SessionHandle {
        let capabilities = HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        };
        let factory: Arc<dyn CoreFactory> = Arc::new(MockFactory);
        let audio_registry = Arc::new(AudioBackendRegistry::new());
        let registry = Arc::new(SystemRegistry::new(vec![factory.clone()]));
        let mut session = SessionHandle::new_ephemeral(capabilities, registry, audio_registry);
        session
            .set_active_system(factory.system_id().as_ref())
            .expect("test setup failed");
        session
    }

    pub(crate) fn test_view(session: &SessionHandle) -> FactorySettingsView {
        let system_id = session.factory().expect("no active system").system_id();
        let snapshot = session.settings_snapshot();
        let language = match snapshot.shared.general.language {
            nerust_settings_core::model::language::AppLanguage::Japanese => {
                nerust_core_traits::factory::settings::Language::Japanese
            }
            nerust_settings_core::model::language::AppLanguage::English => {
                nerust_core_traits::factory::settings::Language::English
            }
            _ => nerust_core_traits::factory::settings::Language::SystemDefault,
        };
        FactorySettingsView {
            language,
            system_config: snapshot.shared.systems.get(system_id.as_ref()).cloned(),
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod session_cov_tests {
    use super::test_util::test_session;

    #[test]
    fn accessors_cover_frame_settings_and_factory() {
        let mut session = test_session();
        assert!(session.render_profile().is_some());
        let _ = session.swap_frame_buffer_tick();
        session.swap_frame_buffer();
        assert!(session.with_frame_buffer(|_| ()).is_some());
        assert!(session.frame_buffer().is_some());
        session.clear_display();
        let _ = session.settings_snapshot();
        let _ = session.chrome();
        let _ = session.transient_policy();
        let _ = session.sidecars();
        let _ = session.host_peripherals();
        let _ = session.current_assignments();
        let _ = session.current_assignments_pairs();
        let _ = session.settings_manager();
        let _ = session.speaker_factory();
        assert!(session.active_factory_cloned().is_some());
        assert!(session.with_active_factory(|_| 7).is_some());
        assert!(session.active_system_display_name().is_some());
        assert!(session.active_system_id_box().is_some());
        assert!(session.active_factory().is_some());
        assert!(!session.registry().all().is_empty());
        assert!(session.factory().is_some());
        assert!(session.active_system_id().is_some());
        assert!(session.default_load_options().is_some());
        let id = session.factory().unwrap().system_id();
        assert!(session.set_active_system(id.as_ref()).is_ok());
        session.set_persistence_backends(
            Box::new(super::fs::FsSlotBackend),
            Box::new(super::fs::FsSlotBackend),
            Box::new(super::fs::FsSlotBackend),
        );
        let tick = session.tick_metrics();
        let _ = session.window_title_cached(&tick).to_string();
        let _ = session
            .notification_history_lines_cached(Default::default())
            .len();
    }

    #[test]
    fn coreless_accessors_report_none() {
        use std::sync::Arc;
        let caps = test_session().capabilities;
        let registry = Arc::new(crate::registry::SystemRegistry::new(vec![]));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let mut session = super::SessionHandle::new_ephemeral(caps, registry, audio);
        assert!(session.render_profile().is_none());
        assert!(session.with_frame_buffer(|_| ()).is_none());
        assert!(session.frame_buffer().is_none());
        session.clear_display();
        session.swap_frame_buffer();
        assert!(session.active_factory_cloned().is_none());
        assert!(session.with_active_factory(|_| 7).is_none());
        assert!(session.active_system_display_name().is_none());
        assert!(session.active_system_id_box().is_none());
        assert!(session.active_factory().is_none());
        assert!(session.factory().is_none());
        assert!(session.active_system_id().is_none());
        assert!(session.default_load_options().is_none());
        let _ = session.current_assignments_pairs();
    }

    #[test]
    fn session_error_conversions_cover_all_legs() {
        use super::SessionError;
        let _ = SessionError::NoCore.to_string();
        let op: SessionError = nerust_emu_thread::OperationError::NoReply.into();
        let _ = op.to_string();
    }
}
