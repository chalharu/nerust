pub mod access;
pub mod commands;
pub mod input;
pub mod lifecycle;
pub mod persistence;
#[cfg(test)]
mod persistence_test;
pub mod title;

use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
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
use nerust_gui_settings::language::AppLanguage;
use nerust_gui_viewmodel::debugger as debug_vm;
use nerust_input_traits::{AttachmentId, DigitalControlId, GuiInput, InputAssignments};
use nerust_keyboard::Key;
use nerust_persistence::{error::PersistenceError, model::StateSlotSummary};
use nerust_render_traits::{FrameBuffer, VideoRenderProfile};
use nerust_settings_core::factory::settings_view;
use nerust_settings_core::i18n as ui_text;
use thiserror::Error;

use crate::{
    emu_core::EmuCore, registry::SystemRegistry, session::persistence::PersistenceManager, settings,
};

struct CoreRuntime {
    emu_core: EmuCore,
    gui_input: GuiInput,
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
    /// Open the debugger window. Not a session command: frontends
    /// route it to their debugger-open path (schema first, so a user
    /// binding wins over the F4 default).
    OpenDebugger,
}

pub struct SessionHandle {
    registry: Arc<SystemRegistry>,
    active_system_id: Option<Box<dyn SystemId>>,
    emu_core: Option<EmuCore>,
    gui_input: Option<GuiInput>,
    current_assignments: InputAssignments,
    field_map: HashMap<(AttachmentId, DigitalControlId), usize>,
    host_peripherals: nerust_core_traits::peripheral::HostPeripheralHandles,
    /// Reverse map: keyboard key → field index, rebuilt on binding/controller change.
    key_field_map: HashMap<nerust_keyboard::Key, usize>,
    capabilities: HostBackendCapabilities,
    settings: SettingsManager,
    settings_snapshot: SettingsSnapshot,
    pressed_keys: BTreeSet<Key>,
    loaded_media: Option<LoadedMedia>,
    persistence: PersistenceManager,
    audio_registry: Arc<AudioBackendRegistry>,
    /// Shell-owned two-phase memory write transaction (Phase B).
    debug_txn: Mutex<debug_vm::WriteTransaction>,
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
    ) -> Result<CoreCreation, SessionError> {
        let speaker = settings::build_speaker(registry, &snapshot.local);
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
                    let fallback_speaker = settings::build_speaker(registry, &fallback.local);
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
                &audio_registry,
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
                InputAssignments { slots: vec![] },
            )
        };
        let mut result = Self {
            emu_core,
            gui_input,
            current_assignments: assignments,
            field_map,
            host_peripherals,
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
            debug_txn: Mutex::new(debug_vm::WriteTransaction::Empty),
        };
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
    ///
    /// On platforms where `ProjectDirs` is unavailable (e.g. Android),
    /// the frontend provides an explicit settings root instead.
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

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            metrics: self
                .emu_core
                .as_ref()
                .map(|c| c.metrics())
                .unwrap_or_default(),
            slots: Arc::from(self.persistence.slots().to_vec()),
            active_slot_id: self.persistence.active_slot_id(),
        }
    }

    // ------------------------------------------------------------------
    // Debugger views (Phase B). Batched, paused-gated reads plus the
    // shell-owned two-phase write transaction. Frontends render from
    // [`debug_vm::DebugSnapshot`] and never touch the core.
    // ------------------------------------------------------------------

    /// Memory-space table for the debugger's space picker. Empty when
    /// no core is loaded.
    pub fn debug_spaces(&self) -> Vec<nerust_core_traits::debugger::SpaceInfo> {
        self.emu_core
            .as_ref()
            .and_then(|core| core.memory_spaces().ok())
            .unwrap_or_default()
    }

    /// Whether the emulation is paused. Snapshot reads require pause;
    /// frontends offer debugger refreshes only while paused.
    pub fn debug_paused(&self) -> bool {
        self.emu_core
            .as_ref()
            .is_none_or(|core| core.metrics().paused)
    }

    /// Single batched snapshot for one drain: one inspect (registers,
    /// dump, panels), one disassembly at `dis_addr` (or the program
    /// counter when `None`), one images pass. `elapsed` measures the
    /// batched core pass. Unavailable when no core is loaded or the
    /// inspect refuses (e.g. running): sections carry degenerates.
    ///
    /// Registers, dump rows, and panels travel structured; text
    /// formatting is the frontends' job at rebuild time. `watch` is
    /// folded into the same inspect when it lands on the fetched
    /// page, saving a thread round-trip in the common case.
    pub fn debug_snapshot(
        &self,
        space: Option<nerust_core_traits::debugger::SpaceId>,
        mem_addr: u32,
        dis_addr: Option<u32>,
        need_images: bool,
        watch: Option<u32>,
    ) -> debug_vm::DebugSnapshot {
        use nerust_core_traits::debugger::{
            DEBUG_DISASM_ROWS, DEBUG_DUMP_PAGE_ROWS, InspectRequest,
        };
        let mut snapshot = debug_vm::DebugSnapshot::default();
        let core = match self.emu_core.as_ref() {
            Some(core) => core,
            None => return snapshot,
        };
        let inspected = match core.inspect(InspectRequest {
            space,
            addr: space.map(|_| mem_addr),
            rows: DEBUG_DUMP_PAGE_ROWS,
        }) {
            Ok(Ok(result)) => result,
            Ok(Err(_)) | Err(_) => return snapshot,
        };
        snapshot.available = true;
        snapshot.regs = inspected
            .registers
            .iter()
            .map(|&(name, value)| (name, value))
            .collect();
        snapshot.reg_names = inspected.registers.iter().map(|(name, _)| *name).collect();
        snapshot.dump_rows = inspected.dump.rows.iter().copied().collect();
        snapshot.panels = inspected.panels.iter().cloned().collect();
        // Anchor without name-scanning: explicit pin wins, otherwise
        // the kernel program counter (one extra crossing, follow mode
        // only).
        let anchor = match dis_addr {
            Some(addr) => Some(addr),
            None => match core.program_counter() {
                Ok(Ok(pc)) => pc,
                _ => None,
            },
        };
        snapshot.pc = anchor;
        snapshot.disasm_lines = match anchor {
            Some(addr) => match core.disassemble(addr, DEBUG_DISASM_ROWS) {
                Ok(Ok(rows)) => rows,
                _ => Vec::new(),
            },
            None => Vec::new(),
        };
        let (images_gen, images) = if need_images {
            match core.debug_images() {
                Ok(Ok((generation, images))) => (generation, images),
                _ => (0, Vec::new()),
            }
        } else {
            (0, Vec::new())
        };
        snapshot.images_gen = images_gen;
        snapshot.images = images.iter().map(debug_vm::rgba_image).collect();
        // Watch folded into the page read when it lands on it;
        // off-page watches still fall back to a single-byte inspect.
        snapshot.watch = watch.and_then(|addr| {
            inspected
                .dump
                .rows
                .iter()
                .find(|row| addr >= row.addr && addr < row.addr + u32::from(row.valid))
                .map(|row| row.bytes[(addr - row.addr) as usize])
        });
        snapshot
    }

    /// Images-only refresh for the live PPU follow while the core
    /// runs. Unlike [`SessionHandle::debug_snapshot`], this never
    /// touches inspect or disassembly: those are paused-gated and come
    /// back degenerate while running, which would blank the dump and
    /// registers the user paused on. The live tick patches only the
    /// returned images (plus the paused/status chrome) and leaves the
    /// last paused sections alone. `None` when imageless or when the
    /// fetch fails.
    pub fn debug_live_images(
        &self,
        need_images: bool,
    ) -> Option<(u64, Vec<(String, u32, u32, Vec<u8>)>)> {
        if !need_images {
            return None;
        }
        let core = self.emu_core.as_ref()?;
        match core.debug_images() {
            Ok(Ok((generation, images))) => Some((
                generation,
                images.iter().map(debug_vm::rgba_image).collect(),
            )),
            _ => None,
        }
    }

    /// Single-byte read for old-value display and watch values.
    pub fn debug_read_byte(
        &self,
        space: nerust_core_traits::debugger::SpaceId,
        addr: u32,
    ) -> Option<u8> {
        let core = self.emu_core.as_ref()?;
        let result = core
            .inspect(nerust_core_traits::debugger::InspectRequest {
                space: Some(space),
                addr: Some(addr),
                rows: 1,
            })
            .ok()?
            .ok()?;
        let row = result.dump.rows.first()?;
        (row.valid > 0).then_some(row.bytes[0])
    }

    /// Single-byte write for freeze re-application. Silent by design:
    /// the rows are the evidence. `false` when unwritable.
    pub fn debug_write_byte(
        &self,
        space: nerust_core_traits::debugger::SpaceId,
        addr: u32,
        value: u8,
    ) -> bool {
        use nerust_core_traits::debugger::MemoryWrite;
        let core = match self.emu_core.as_ref() {
            Some(core) => core,
            None => return false,
        };
        matches!(
            core.write_memory(MemoryWrite {
                space,
                addr,
                width: 1,
                value: u64::from(value),
            }),
            Ok(Ok(()))
        )
    }

    /// Single-register write for the register viewer. Status
    /// strings follow the memory-commit shape, in the drain language.
    pub fn debug_write_register(
        &self,
        language: AppLanguage,
        name: &'static str,
        value: u64,
    ) -> String {
        use nerust_settings_core::i18n::UiText;
        let core = match self.emu_core.as_ref() {
            Some(core) => core,
            None => return ui_text::text(language, UiText::NoCore).to_string(),
        };
        match core.write_register(name, value) {
            Ok(Ok(())) => ui_text::register_wrote(language, name, value),
            Ok(Err(error)) => ui_text::register_refused(language, &format!("{error:?}")),
            Err(error) => ui_text::thread_failed(language, &format!("{error:?}")),
        }
    }

    /// Select a row: read the old byte and prepare the transaction,
    /// overwriting any state.
    pub fn debug_prepare_write(
        &self,
        space: nerust_core_traits::debugger::SpaceId,
        addr: u32,
    ) -> Option<u8> {
        let old = self.debug_read_byte(space, addr);
        self.debug_txn.lock().unwrap().prepare(space, addr, old);
        old
    }

    /// Stage the parsed value; only a prepared transaction accepts it.
    pub fn debug_stage_write(&self, value: u64) {
        self.debug_txn.lock().unwrap().stage(value);
    }

    /// Confirm-row text, if fully staged. Single formatter shared with
    /// the optimistic display, in the drain language.
    pub fn debug_pending_text(&self, language: AppLanguage) -> Option<String> {
        self.debug_txn.lock().unwrap().pending_text(language)
    }

    /// Commit the staged write, consuming the transaction. Status
    /// strings match the verified prototype pixels exactly.
    pub fn debug_commit_write(&self, language: AppLanguage) -> String {
        use nerust_core_traits::debugger::MemoryWrite;
        use nerust_settings_core::i18n::UiText;
        let staged = self.debug_txn.lock().unwrap().take_commit();
        let (space, addr, value) = match staged {
            Some(staged) => staged,
            None => return ui_text::text(language, UiText::NothingToWrite).to_string(),
        };
        let core = match self.emu_core.as_ref() {
            Some(core) => core,
            None => return ui_text::text(language, UiText::NoCore).to_string(),
        };
        match core.write_memory(MemoryWrite {
            space,
            addr,
            width: 1,
            value,
        }) {
            Ok(Ok(())) => ui_text::wrote_memory(language, addr, value),
            Ok(Err(error)) => ui_text::write_refused(language, &format!("{error:?}")),
            Err(error) => ui_text::thread_failed(language, &format!("{error:?}")),
        }
    }

    /// Drop any uncommitted transaction (cancel, space switch).
    pub fn debug_clear_write(&self) {
        self.debug_txn.lock().unwrap().clear();
    }

    /// Report drain traffic: `state_changing` kills staged-but-
    /// uncommitted values from previous drains (step, refresh, pause
    /// switches). Select/stage/commit traffic passes `false`: it
    /// builds the transaction this drain consumes. The frontend owns
    /// request semantics; the session owns the kill.
    pub fn debug_note_traffic(&self, state_changing: bool) {
        if state_changing {
            self.debug_txn.lock().unwrap().on_state_changing_traffic();
        }
    }

    /// Pause for debugger control. Status strings match the verified
    /// prototype pixels.
    pub fn debug_pause(&self, language: AppLanguage) -> String {
        use nerust_settings_core::i18n::UiText;
        match self.emu_core.as_ref() {
            Some(core) => match core.pause() {
                Ok(()) => ui_text::text(language, UiText::PausedState).to_string(),
                Err(error) => ui_text::pause_failed(language, &format!("{error:?}")),
            },
            None => ui_text::text(language, UiText::NoCore).to_string(),
        }
    }

    /// Resume for debugger control.
    pub fn debug_resume(&self, language: AppLanguage) -> String {
        use nerust_settings_core::i18n::UiText;
        match self.emu_core.as_ref() {
            Some(core) => match core.resume() {
                Ok(()) => ui_text::text(language, UiText::Resume).to_string(),
                Err(error) => ui_text::resume_failed(language, &format!("{error:?}")),
            },
            None => ui_text::text(language, UiText::NoCore).to_string(),
        }
    }

    /// Advance one unit for debugger control. Returns the stepped
    /// count (`stepped N`) or a loud failure.
    pub fn debug_step(
        &self,
        language: AppLanguage,
        unit: nerust_core_traits::debugger::StepUnit,
    ) -> String {
        use nerust_settings_core::i18n::UiText;
        let core = match self.emu_core.as_ref() {
            Some(core) => core,
            None => return ui_text::text(language, UiText::NoCore).to_string(),
        };
        match core.step(unit) {
            Ok(Ok(count)) => ui_text::stepped(language, count),
            Ok(Err(error)) => ui_text::step_failed(language, &format!("{error:?}")),
            Err(error) => ui_text::thread_failed(language, &format!("{error:?}")),
        }
    }

    pub fn render_profile(&self) -> Option<&VideoRenderProfile> {
        self.emu_core.as_ref().map(|c| c.render_profile())
    }

    pub fn swap_frame_buffer(&mut self) {
        if let Some(ref mut gui_input) = self.gui_input {
            gui_input.publish();
        }
        if let Some(ref mut core) = self.emu_core {
            core.swap_frame_buffer();
        }
    }

    pub fn frame_buffer(&self) -> Option<&FrameBuffer> {
        self.emu_core.as_ref().map(|c| c.frame_buffer())
    }

    pub fn clear_display(&mut self) {
        if let Some(ref mut core) = self.emu_core {
            core.clear_display();
        }
    }

    pub fn settings_snapshot(&self) -> &SettingsSnapshot {
        &self.settings_snapshot
    }

    pub fn host_peripherals(&self) -> &nerust_core_traits::peripheral::HostPeripheralHandles {
        &self.host_peripherals
    }

    pub fn current_assignments(&self) -> &InputAssignments {
        &self.current_assignments
    }

    #[cfg(test)]
    pub(crate) fn loaded_media_ref(&self) -> &Option<LoadedMedia> {
        &self.loaded_media
    }

    pub fn settings_manager(&self) -> &SettingsManager {
        &self.settings
    }

    pub fn set_persistence_backends(
        &mut self,
        slot_backend: Box<dyn persistence::SlotBackend>,
        autosave_backend: Box<dyn persistence::AutoSaveBackend>,
        mapper_backend: Box<dyn persistence::MapperSaveBackend>,
    ) {
        self.persistence =
            PersistenceManager::with_all_backends(slot_backend, autosave_backend, mapper_backend);
    }

    #[cfg(test)]
    pub(crate) fn set_settings(&mut self, settings: SettingsManager) {
        self.settings = settings;
    }

    #[cfg(test)]
    pub(crate) fn set_settings_snapshot(&mut self, snapshot: SettingsSnapshot) {
        self.settings_snapshot = snapshot;
    }

    pub fn active_factory(&self) -> Option<&Arc<dyn CoreFactory>> {
        self.active_system_id
            .as_ref()
            .and_then(|id| self.registry.find_by_id(id.as_ref()))
    }

    pub fn registry(&self) -> &SystemRegistry {
        &self.registry
    }

    pub fn factory(&self) -> Option<&dyn CoreFactory> {
        self.active_factory().map(|a| &**a)
    }

    pub fn active_system_id(&self) -> Option<&dyn SystemId> {
        self.active_system_id.as_deref()
    }

    pub fn current_assignments_pairs(&self) -> Vec<(String, Option<String>)> {
        self.current_assignments.to_string_pairs()
    }

    pub fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
        self.active_factory().map(|f| f.default_load_options())
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

use crate::{
    load::{RomLoadTarget, RomLoaderError, SystemActivationError},
    session::commands::SessionCommand,
};

impl RomLoadTarget for SessionHandle {
    fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
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

    /// Notifies the session of the detected system for the ROM being loaded.
    ///
    /// Called by `RegistryRomLoader` before loading the ROM into the session.
    /// Rebuilds the `EmuCore` immediately with the correct factory so that
    /// `load_resolved` (the next call) can load ROM data into a properly
    /// configured core. This is the core entry-point for lazy system activation.
    fn set_active_system(&mut self, system_id: &dyn SystemId) -> Result<(), SystemActivationError> {
        if self.active_system_id.as_ref().map(|x| x.as_ref()) == Some(system_id) {
            return Ok(());
        }
        let factory = self
            .registry
            .find_by_id(system_id)
            .cloned()
            .ok_or(SystemActivationError::NotRegistered(system_id.clone_box()))?;
        let requested_assignments =
            Self::load_assignments(&factory, &self.settings_snapshot, system_id);
        let created = Self::create_core_with_assignments(
            &factory,
            &self.audio_registry,
            &self.settings_snapshot,
            &requested_assignments,
        )
        .map_err(|e| SystemActivationError::Activation(e.to_string()))?;

        if let Some(ref core) = self.emu_core {
            self.persistence
                .flush_mapper_save(core)
                .map_err(|e| SystemActivationError::Activation(e.to_string()))?;
        }

        self.active_system_id = Some(system_id.clone_box());
        self.current_assignments = created.applied_assignments;
        self.emu_core = Some(created.runtime.emu_core);
        self.gui_input = Some(created.runtime.gui_input);
        self.field_map = created.runtime.field_map;
        self.host_peripherals = created.runtime.host_peripherals;
        self.loaded_media = None;
        self.persistence.reset();
        self.pressed_keys.clear();
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
    use crate::{load::RomLoadTarget, registry::SystemRegistry, session::SessionHandle};

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
            nerust_gui_settings::language::AppLanguage::Japanese => {
                nerust_core_traits::factory::settings::Language::Japanese
            }
            nerust_gui_settings::language::AppLanguage::English => {
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
