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
use nerust_settings_core::factory::settings_view;
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

// ---------------------------------------------------------------------------
// SPIKE (iteration 6, DO NOT MERGE): debugger-window probes and shared
// text formatters. Deleted with the spike branch.
// ---------------------------------------------------------------------------

/// SPIKE: format one inspect dump as plain hex text.
pub fn spike_format_dump(dump: &nerust_core_traits::debugger::MemoryDump) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for row in dump.rows.iter() {
        let _ = write!(out, "{:04X}:", row.addr);
        for i in 0..row.valid {
            let _ = write!(out, " {:02X}", row.bytes[i as usize]);
        }
        out.push('\n');
    }
    out
}

/// SPIKE: format disassembly rows; the PC row carries a `>` marker.
pub fn spike_format_disasm(rows: &[nerust_core_traits::debugger::DisasmLine]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for row in rows {
        let mark = if row.is_pc { '>' } else { ' ' };
        let _ = write!(out, "{mark}{:04X}: ", row.addr);
        for i in 0..row.len {
            let _ = write!(out, "{:02X} ", row.bytes[i as usize]);
        }
        for _ in row.len..3 {
            out.push_str("   ");
        }
        out.push_str(&row.text);
        out.push('\n');
    }
    out
}

impl SessionHandle {
    /// SPIKE: pause the running core (debug precondition).
    pub fn spike_pause(&self) -> String {
        match self.emu_core.as_ref() {
            Some(core) => match core.pause() {
                Ok(()) => "paused".to_string(),
                Err(e) => format!("pause failed: {e:?}"),
            },
            None => "no core".to_string(),
        }
    }

    /// SPIKE: list memory spaces generically (no per-system naming).
    pub fn spike_memory_spaces(&self) -> Vec<nerust_core_traits::debugger::SpaceInfo> {
        match self.emu_core.as_ref() {
            Some(core) => core.memory_spaces().unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// SPIKE: read `rows` of `space` at `addr` as shared-format text.
    pub fn spike_read_text(
        &self,
        space: nerust_core_traits::debugger::SpaceId,
        addr: u32,
        rows: u16,
    ) -> String {
        let core = match self.emu_core.as_ref() {
            Some(core) => core,
            None => return "no core".to_string(),
        };
        let req = nerust_core_traits::debugger::InspectRequest {
            space: Some(space),
            addr: Some(addr),
            rows,
        };
        match core.inspect(req) {
            Ok(Ok(result)) => spike_format_dump(&result.dump),
            Ok(Err(e)) => format!("inspect failed: {e:?}"),
            Err(e) => format!("thread failed: {e:?}"),
        }
    }

    /// SPIKE: registers as `name: $xxxx` lines in kernel order.
    pub fn spike_registers_text(&self) -> String {
        let core = match self.emu_core.as_ref() {
            Some(core) => core,
            None => return "no core".to_string(),
        };
        let req = nerust_core_traits::debugger::InspectRequest {
            space: None,
            addr: None,
            rows: 0,
        };
        match core.inspect(req) {
            Ok(Ok(result)) => {
                use std::fmt::Write as _;
                let mut out = String::new();
                for (name, value) in result.registers.iter() {
                    let _ = writeln!(out, "{name}: ${value:04X}");
                }
                out
            }
            Ok(Err(e)) => format!("inspect failed: {e:?}"),
            Err(e) => format!("thread failed: {e:?}"),
        }
    }

    /// SPIKE: current PC, if the register list carries one.
    pub fn spike_pc(&self) -> Option<u32> {
        let core = self.emu_core.as_ref()?;
        let req = nerust_core_traits::debugger::InspectRequest {
            space: None,
            addr: None,
            rows: 0,
        };
        let result = core.inspect(req).ok()?.ok()?;
        result
            .registers
            .iter()
            .find(|(name, _)| *name == "pc")
            .map(|(_, v)| *v as u32)
    }

    /// SPIKE: disassembly window text, PC-centered when known.
    /// No system-specific fallback address: without a PC there is no
    /// generic anchor, so the window says so.
    pub fn spike_disasm_text(&self, count: u16) -> String {
        let core = match self.emu_core.as_ref() {
            Some(core) => core,
            None => return "no core".to_string(),
        };
        let Some(addr) = self.spike_pc() else {
            return "(no PC register)".to_string();
        };
        match core.disassemble(addr, count) {
            Ok(Ok(rows)) if !rows.is_empty() => spike_format_disasm(&rows),
            Ok(Ok(_)) => "(no disassembly)".to_string(),
            Ok(Err(e)) => format!("disassemble failed: {e:?}"),
            Err(e) => format!("thread failed: {e:?}"),
        }
    }

    /// SPIKE: disassemble at an explicit address (free navigation).
    pub fn spike_disasm_at(&self, addr: u32, count: u16) -> String {
        let core = match self.emu_core.as_ref() {
            Some(core) => core,
            None => return "no core".to_string(),
        };
        match core.disassemble(addr, count) {
            Ok(Ok(rows)) if !rows.is_empty() => spike_format_disasm(&rows),
            Ok(Ok(_)) => "(no disassembly)".to_string(),
            Ok(Err(e)) => format!("disassemble failed: {e:?}"),
            Err(e) => format!("thread failed: {e:?}"),
        }
    }

    /// SPIKE: resume the paused core.
    pub fn spike_resume(&self) -> String {
        match self.emu_core.as_ref() {
            Some(core) => match core.resume() {
                Ok(()) => "resumed".to_string(),
                Err(e) => format!("resume failed: {e:?}"),
            },
            None => "no core".to_string(),
        }
    }

    /// SPIKE: step one frame (debug precondition: paused).
    pub fn spike_step_frame(&self) -> String {
        self.spike_step(nerust_core_traits::debugger::StepUnit::Frame)
    }

    /// SPIKE: step one instruction (debug precondition: paused).
    pub fn spike_step_instruction(&self) -> String {
        self.spike_step(nerust_core_traits::debugger::StepUnit::Instruction)
    }

    fn spike_step(&self, unit: nerust_core_traits::debugger::StepUnit) -> String {
        let core = match self.emu_core.as_ref() {
            Some(core) => core,
            None => return "no core".to_string(),
        };
        match core.step(unit) {
            Ok(Ok(count)) => format!("stepped {count}"),
            Ok(Err(e)) => format!("step failed: {e:?}"),
            Err(e) => format!("thread failed: {e:?}"),
        }
    }
}

/// SPIKE: parse a hex address (`0010`, `0x0010`, `$0010`).
pub fn spike_parse_hex_addr(raw: &str) -> Option<u32> {
    let text = raw.trim();
    let text = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .or_else(|| text.strip_prefix('$'))
        .unwrap_or(text);
    if text.is_empty() {
        return None;
    }
    u32::from_str_radix(text, 16).ok()
}

// SPIKE (iteration 7, DO NOT MERGE): throwaway helper tests.
#[cfg(test)]
mod spike_nav_tests {
    use super::spike_parse_hex_addr;

    #[test]
    fn spike_parse_hex_addr_accepts_plain_0x_and_dollar() {
        assert_eq!(spike_parse_hex_addr("0010"), Some(0x10));
        assert_eq!(spike_parse_hex_addr("0xC290"), Some(0xC290));
        assert_eq!(spike_parse_hex_addr("$c290"), Some(0xC290));
        assert_eq!(spike_parse_hex_addr("  8000  "), Some(0x8000));
        assert_eq!(spike_parse_hex_addr(""), None);
        assert_eq!(spike_parse_hex_addr("zz"), None);
    }
}
