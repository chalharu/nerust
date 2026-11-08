use std::{collections::HashSet, path::Path, sync::Arc};

use nerust_core_traits::{audio::AudioBackendRegistry, identity::SystemId};
use nerust_gui_application::{
    EmulationPort, FramePort, HistoryPort, KeyboardPort, MediaPort, SessionAction,
    SessionActionPort, SettingsPort,
};
use nerust_gui_runtime::settings::{HostBackendCapabilities, SettingsPaths, SettingsSnapshot};
use nerust_gui_shell::{
    load::{LoadEnvironment, RomLoadTarget, RomLoader, RomLoaderError, SystemActivationError},
    registry::SystemRegistry,
    session::{KeyboardShortcut, SessionHandle, access::PauseControl, commands::SessionCommand},
};
use nerust_render_traits::{FrameBuffer, VideoRenderProfile};
use nerust_system_traits::factory::load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest};

pub struct SessionAdapter {
    session: SessionHandle,
    media_revision: u64,
    loader: Box<dyn RomLoader>,
    held: bool,
    pressed_keys: HashSet<nerust_input_traits::Key>,
    audio: Arc<AudioBackendRegistry>,
    osd: nerust_gui_shell::session::osd::OsdManager,
}

impl SessionAdapter {
    pub(crate) fn media_revision(&self) -> u64 {
        self.media_revision
    }

    /// Narrow settings write: remembers the main window size for the next
    /// launch. Called at exit; failures only skip the restore.
    pub fn remember_window_size(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.session
            .update_window_size(width, height)
            .map_err(|error| error.to_string())
    }

    /// Test-only access to the shell handle (pause/resume/reads).
    /// Production reaches the session through the typed port traits.
    #[cfg(test)]
    pub(crate) fn handle(&self) -> &nerust_gui_shell::session::SessionHandle {
        &self.session
    }

    pub(crate) fn ppu_live_begin(
        &self,
        generation: Option<u64>,
    ) -> nerust_gui_shell::debug::LiveImagesFlight {
        self.session.debug_live_images_begin_since(generation)
    }
    pub(crate) fn debug_spaces(&self) -> Vec<nerust_core_traits::debugger::SpaceInfo> {
        self.session.debug_spaces()
    }

    pub(crate) fn debug_begin(
        &self,
        actions: &[nerust_gui_shell::debug::DebugAction],
        input: &nerust_gui_shell::debug::DebugDrainInput<'_>,
    ) -> nerust_gui_shell::debug::DrainFlight {
        self.session.debug_drain_begin(actions, input)
    }

    pub(crate) fn menu_slots(
        &self,
    ) -> (
        Arc<[nerust_persistence::model::StateSlotSummary]>,
        Option<u64>,
    ) {
        (self.session.slots_arc(), self.session.active_slot_id())
    }

    pub(crate) fn debugger_layout_begin(&self) -> (String, nerust_gui_shell::debug::DrainFlight) {
        self.debug_layout_begin(false)
    }

    pub(crate) fn ppu_layout_begin(&self) -> nerust_gui_shell::debug::DrainFlight {
        self.debug_layout_begin(true).1
    }

    fn debug_layout_begin(
        &self,
        need_images: bool,
    ) -> (String, nerust_gui_shell::debug::DrainFlight) {
        use nerust_gui_shell::debug::DebugAction;
        let spaces = self.session.debug_spaces();
        let space = spaces.first();
        let input = nerust_gui_shell::debug::DebugDrainInput {
            space: space.map(|space| space.id),
            mem_addr: space.map(|space| *space.range.start()).unwrap_or(0),
            dis_addr: None,
            watch: None,
            reg_target: None,
            freeze: None,
            prev_dump_rows: &[],
            need_images,
        };
        // The debugger owns no pause: opening never pauses, and closing
        // never resumes. A running core serves a Running degenerate until
        // an external pause (Focus, menu, shortcut) lands, at which point
        // the pane re-snapshots on the pause edge. Images stay with the
        // PPU viewer: CHR decode is skipped here, not just unused.
        let actions = vec![DebugAction::Refresh];
        (
            space
                .map(|space| space.name.to_string())
                .unwrap_or_default(),
            self.session.debug_drain_begin(&actions, &input),
        )
    }

    /// W3C code (`NumpadEnter`) to settings name (`numpad_enter`): matches the
    /// `snake_case` serde vocabulary of [`nerust_input_traits::Key`]. Upstream
    /// naming drift is absorbed one layer down: `KeyCode::from_name` aliases
    /// (`MetaLeft` → `SuperLeft`) before this ever runs.
    fn key_name(code: &panechord::KeyCode) -> String {
        let mut name = String::new();
        for (index, character) in code.as_str().chars().enumerate() {
            if index > 0 && character.is_ascii_uppercase() {
                name.push('_');
            }
            name.push(character.to_ascii_lowercase());
        }
        name
    }

    pub(crate) fn key(code: &panechord::KeyCode) -> Option<nerust_input_traits::Key> {
        use serde::Deserialize;
        nerust_input_traits::Key::deserialize(serde::de::value::StrDeserializer::<
            serde::de::value::Error,
        >::new(&Self::key_name(code)))
        .ok()
    }

    pub(crate) fn session_action(
        action: nerust_settings_core::model::input::ShortcutAction,
    ) -> Option<SessionAction> {
        use nerust_settings_core::model::input::ShortcutAction;
        Some(match action {
            ShortcutAction::Reset => SessionAction::Reset,
            ShortcutAction::SaveActiveSlot => SessionAction::SaveState,
            ShortcutAction::LoadActiveSlot => SessionAction::LoadState,
            ShortcutAction::SelectNextSlot => SessionAction::NextSlot,
            ShortcutAction::SelectPreviousSlot => SessionAction::PreviousSlot,
            ShortcutAction::TogglePause
            | ShortcutAction::ToggleFullscreen
            | ShortcutAction::OpenDebugger => return None,
        })
    }

    pub fn new(
        registry: Arc<SystemRegistry>,
        audio: Arc<AudioBackendRegistry>,
        loader: Box<dyn RomLoader>,
        capabilities: HostBackendCapabilities,
        settings_paths: Option<SettingsPaths>,
    ) -> Result<Self, String> {
        let session = match settings_paths {
            Some(paths) => {
                SessionHandle::new_with_settings_paths(capabilities, registry, audio.clone(), paths)
            }
            None => SessionHandle::new(capabilities, registry, audio.clone()),
        }
        .map_err(|error| error.to_string())?;
        Ok(Self {
            session,
            media_revision: 0,
            loader,
            held: false,
            pressed_keys: HashSet::new(),
            audio,
            osd: nerust_gui_shell::session::osd::OsdManager::new(),
        })
    }

    pub fn render_profile(&self) -> Option<VideoRenderProfile> {
        self.session.render_profile().cloned()
    }

    pub fn frame_counter(&self) -> u64 {
        self.session.tick_metrics().frame_counter
    }

    pub fn factories(&self) -> &[Arc<dyn nerust_system_traits::factory::CoreFactory>] {
        self.session.factories()
    }

    pub fn supported_sample_rates(&self) -> Arc<[u32]> {
        self.audio.supported_rates().iter().copied().collect()
    }

    pub fn osd_state(&self) -> &nerust_gui_shell::session::osd::OsdManager {
        &self.osd
    }

    /// Take the next modal batch for display. The Shell queue stays the only
    /// queue gate; the desktop dialog coordinator only moves batches.
    pub fn take_modal_batch(&mut self) -> Option<String> {
        self.osd.take_modal_batch()
    }

    /// Ack the showing dialog. Releases the queue gate for the next batch.
    pub fn modal_closed(&mut self) {
        self.osd.modal_closed()
    }

    pub fn metrics(&self) -> nerust_gui_shell::session::osd::TickMetrics {
        Self::display_metrics(self.session.tick_metrics(), self.held)
    }

    fn display_metrics(
        mut tick: nerust_gui_shell::session::osd::TickMetrics,
        held: bool,
    ) -> nerust_gui_shell::session::osd::TickMetrics {
        tick.paused |= held;
        tick
    }

    pub fn chrome(&self) -> nerust_gui_shell::session::osd::OsdChrome {
        self.session.chrome()
    }

    pub(crate) fn history_revision(&self) -> u64 {
        self.session.notification_history_seq()
    }

    pub(crate) fn history_lines(
        &self,
        cache: &mut nerust_gui_shell::session::session_query::HistoryView,
        language: nerust_settings_core::model::language::AppLanguage,
    ) -> Vec<nerust_gui_shell::session::osd::HistoryLine> {
        cache
            .lines(
                self.session.notification_history(),
                self.history_revision(),
                language,
            )
            .to_vec()
    }
}

impl EmulationPort for SessionAdapter {
    fn loaded(&self) -> bool {
        self.session.tick_metrics().loaded
    }
    fn paused(&self) -> bool {
        self.session.tick_metrics().paused
    }
    fn set_execution_held(&mut self, held: bool) {
        self.held = held;
        self.session.set_execution_held(held);
    }
    fn pause(&mut self) {
        KeyboardPort::clear_input(self);
        PauseControl::pause(&mut self.session);
    }
    fn resume(&mut self) {
        PauseControl::resume(&mut self.session);
    }
    fn flush(&mut self) {
        KeyboardPort::clear_input(self);
        self.session.flush_before_exit();
    }
}

impl HistoryPort for SessionAdapter {
    fn clear_history(&mut self) {
        self.session.clear_notification_history();
    }
}

impl KeyboardPort for SessionAdapter {
    type Key = nerust_input_traits::Key;
    type Shortcut = KeyboardShortcut;

    fn key_event(
        &mut self,
        key: Self::Key,
        pressed: bool,
        controller_enabled: bool,
    ) -> Option<Self::Shortcut> {
        if pressed {
            if !self.pressed_keys.insert(key) {
                return None;
            }
        } else {
            self.pressed_keys.remove(&key);
        }
        let shortcut = self.session.handle_keyboard_key(key, pressed);
        if !controller_enabled {
            self.session.clear_input();
        }
        shortcut
    }

    fn clear_input(&mut self) {
        self.pressed_keys.clear();
        self.session.clear_input();
    }
}

impl FramePort for SessionAdapter {
    type Frame = FrameBuffer;
    fn advance_frame(&mut self) {
        self.session.swap_frame_buffer();
        let input = nerust_gui_shell::session::osd::PushInput {
            policy: self.session.transient_policy(),
            language: self.session.settings_snapshot().shared.general.language,
            now: std::time::Instant::now(),
            tick: self.metrics(),
        };
        self.session.pump_osd(&mut self.osd, input);
    }
    fn frame(&self) -> Option<&FrameBuffer> {
        self.session.frame_buffer()
    }
}

impl SessionActionPort for SessionAdapter {
    fn execute(&mut self, action: SessionAction) -> Result<bool, String> {
        if let SessionAction::SelectSlot(id)
        | SessionAction::SaveSlot(id)
        | SessionAction::LoadSlot(id)
        | SessionAction::DeleteSlot(id) = action
            && !self.session.slots().iter().any(|slot| slot.slot_id == id)
        {
            return Ok(false);
        }
        if action == SessionAction::Unload {
            self.media_revision = self
                .media_revision
                .checked_add(1)
                .ok_or_else(|| "media revision exhausted".to_owned())?;
            return self
                .session
                .unload()
                .map(|_| true)
                .map_err(|error| error.to_string());
        }
        let command = match action {
            SessionAction::Reset => SessionCommand::Reset,
            SessionAction::Unload => unreachable!(),
            SessionAction::SaveState => SessionCommand::SaveActiveSlotOrNew,
            SessionAction::LoadState => SessionCommand::LoadActiveSlot,
            SessionAction::CreateSlot => SessionCommand::CreateSlot,
            SessionAction::SelectSlot(id) => SessionCommand::SelectActiveSlot(id),
            SessionAction::SaveSlot(id) => SessionCommand::SaveSlot(id),
            SessionAction::LoadSlot(id) => SessionCommand::LoadSlot(id),
            SessionAction::DeleteSlot(id) => SessionCommand::DeleteSlot(id),
            SessionAction::NextSlot => SessionCommand::SelectNextSlot,
            SessionAction::PreviousSlot => SessionCommand::SelectPreviousSlot,
        };
        self.session
            .run_command(command)
            .map(|outcome| outcome.executed)
            .map_err(|error| error.to_string())
    }
}

impl SettingsPort for SessionAdapter {
    type Settings = SettingsSnapshot;
    fn settings(&self) -> &SettingsSnapshot {
        self.session.settings_snapshot()
    }
    fn apply_settings(&mut self, settings: SettingsSnapshot) -> Result<(), String> {
        self.session
            .apply_settings(settings)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

struct LoadTarget<'a> {
    session: &'a mut SessionHandle,
    held: bool,
}

impl LoadEnvironment for LoadTarget<'_> {
    fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
        self.session.default_load_options()
    }
    fn settings_snapshot(&self) -> &SettingsSnapshot {
        self.session.settings_snapshot()
    }
    fn set_active_system(&mut self, id: &dyn SystemId) -> Result<(), SystemActivationError> {
        self.session.set_active_system(id)
    }
}

impl RomLoadTarget for LoadTarget<'_> {
    fn load_resolved(
        &mut self,
        media: MediaObject,
        options: ResolvedLoadRequest,
    ) -> Result<(), RomLoaderError> {
        self.session
            .load_resolved(media, options)
            .map_err(RomLoaderError::from_session)
    }
    fn resume(&mut self) {
        if !self.held {
            PauseControl::resume(self.session);
        }
    }
}

impl MediaPort for SessionAdapter {
    fn load(&mut self, path: &Path) -> Result<(), String> {
        self.media_revision = self
            .media_revision
            .checked_add(1)
            .ok_or_else(|| "media revision exhausted".to_owned())?;
        self.loader
            .load_rom(
                path,
                &mut LoadTarget {
                    session: &mut self.session,
                    held: self.held,
                },
            )
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
pub(crate) mod test_fixture {
    //! Shared headless fixture: a real `SessionAdapter` (NES core) with an
    //! isolated settings root and an in-test iNES ROM. Panes own the driving;
    //! this module only builds the loaded adapter and cleans the temp dir.
    use super::SessionAdapter;
    use nerust_gui_application::MediaPort;
    use nerust_gui_runtime::settings::{
        HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    pub(crate) struct TempDir {
        path: PathBuf,
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    pub(crate) fn test_ines() -> Vec<u8> {
        let mut data = vec![0x4E, 0x45, 0x53, 0x1A, 2u8, 1, 0, 0];
        data.resize(16 + 0x8000 + 0x2000, 0);
        // Point the reset vector into PRG (0x8000): a zero reset leaves the
        // program counter outside PRG, where disassembly is legitimately empty.
        data[16 + 0x7FFC] = 0x00;
        data[16 + 0x7FFD] = 0x80;
        data
    }

    /// Loaded adapter plus its temp dir (drop cleans up). The core runs;
    /// panes pause/step as their flow requires.
    pub(crate) fn loaded_adapter() -> (SessionAdapter, TempDir) {
        loaded_adapter_with_rom(test_ines())
    }

    /// Loaded adapter for a caller-supplied iNES image (jump targets,
    /// bank layouts, and other byte-level shapes the shared ROM lacks).
    pub(crate) fn loaded_adapter_with_rom(image: Vec<u8>) -> (SessionAdapter, TempDir) {
        // Nanos alone can collide across parallel test threads in one
        // process; the counter makes each fixture directory unique.
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nerust-desktop-ops-{}-{nonce}-{count}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
            Arc::new(nerust_nes_factory::NesFactory),
        ]));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::default());
        let loader = registry
            .create_loader(HashMap::new())
            .expect("registered NES loader");
        let capabilities = HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: true,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        };
        let mut adapter = SessionAdapter::new(
            registry,
            audio,
            loader,
            capabilities,
            Some(SettingsPaths::from_root(dir.join("settings"))),
        )
        .expect("adapter with temp settings root");
        let rom = dir.join("test.nes");
        std::fs::write(&rom, image).expect("fixture ROM should write");
        adapter.load(&rom).expect("fixture ROM loads");
        (adapter, TempDir { path: dir })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_gui_shell::session::osd::TickMetrics;

    #[test]
    fn display_pause_includes_hold_without_overwriting_worker_metrics() {
        let tick = TickMetrics {
            fps: 59.9,
            loaded: true,
            frame_counter: 123,
            paused: false,
        };
        let held = SessionAdapter::display_metrics(tick, true);
        assert!(held.paused);
        assert_eq!(held.fps, tick.fps);
        assert_eq!(held.loaded, tick.loaded);
        assert_eq!(held.frame_counter, tick.frame_counter);
        assert!(!SessionAdapter::display_metrics(tick, false).paused);
        assert!(
            SessionAdapter::display_metrics(
                TickMetrics {
                    paused: true,
                    ..tick
                },
                false
            )
            .paused
        );
    }

    #[test]
    fn normalized_keys_bridge_to_existing_settings_without_a_frontend_key_table() {
        for (name, expected) in [
            ("KeyV", nerust_input_traits::Key::KeyV),
            ("F11", nerust_input_traits::Key::F11),
            ("IntlYen", nerust_input_traits::Key::IntlYen),
            ("MetaLeft", nerust_input_traits::Key::SuperLeft),
            ("NumpadEnter", nerust_input_traits::Key::NumpadEnter),
        ] {
            assert_eq!(
                SessionAdapter::key(&panechord::KeyCode::from_name(name)),
                Some(expected)
            );
        }
        assert_eq!(SessionAdapter::key(&panechord::KeyCode::Unidentified), None);
    }

    #[test]
    fn key_name_conversion_pins_camel_to_snake_rules() {
        // The serde vocabulary is the contract; this pins the mechanical
        // conversion (letter groups, digits, multi-capital acronyms) so a
        // drift shows up here rather than as a silent unbound key.
        for (code, expected) in [
            ("KeyV", "key_v"),
            ("F11", "f11"),
            ("ArrowUp", "arrow_up"),
            ("IntlYen", "intl_yen"),
            ("NumpadEnter", "numpad_enter"),
            ("Unidentified", "unidentified"),
        ] {
            assert_eq!(
                SessionAdapter::key_name(&panechord::KeyCode::from_name(code)),
                expected,
                "{code}"
            );
        }
    }

    #[test]
    fn window_size_write_restores_on_next_launch() {
        // Narrow settings write: the exit-time size restores verbatim.
        let (mut adapter, _dir) = test_fixture::loaded_adapter();
        adapter
            .remember_window_size(800, 600)
            .expect("size persists");
        assert_eq!(
            adapter.settings().app_state.window_size("main"),
            Some(
                nerust_settings_core::model::app_state::RememberedWindowSize {
                    width: 800,
                    height: 600,
                }
            )
        );
    }

    #[test]
    fn adapter_boots_without_explicit_settings_paths() {
        // Default-paths branch: no fixture root, core stays unloaded.
        use std::collections::HashMap;
        let registry = std::sync::Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
            std::sync::Arc::new(nerust_nes_factory::NesFactory),
        ]));
        let audio = std::sync::Arc::new(nerust_core_traits::audio::AudioBackendRegistry::default());
        let loader = registry
            .create_loader(HashMap::new())
            .expect("registered NES loader");
        let capabilities = nerust_gui_runtime::settings::HostBackendCapabilities {
            window: nerust_gui_runtime::settings::HostWindowCapabilities {
                remembers_window_size: true,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        };
        let adapter = SessionAdapter::new(registry, audio, loader, capabilities, None)
            .expect("default paths boot");
        assert!(!adapter.loaded());
        assert!(!adapter.paused());
        assert_eq!(adapter.frame_counter(), 0);
    }

    #[test]
    fn session_action_maps_shortcuts_and_defers_chrome_keys() {
        // Mapped actions reach the shell; chrome keys stay frontend-owned.
        use nerust_settings_core::model::input::ShortcutAction;
        for (action, expected) in [
            (ShortcutAction::Reset, Some(SessionAction::Reset)),
            (
                ShortcutAction::SaveActiveSlot,
                Some(SessionAction::SaveState),
            ),
            (
                ShortcutAction::LoadActiveSlot,
                Some(SessionAction::LoadState),
            ),
            (
                ShortcutAction::SelectNextSlot,
                Some(SessionAction::NextSlot),
            ),
            (
                ShortcutAction::SelectPreviousSlot,
                Some(SessionAction::PreviousSlot),
            ),
            (ShortcutAction::TogglePause, None),
            (ShortcutAction::ToggleFullscreen, None),
            (ShortcutAction::OpenDebugger, None),
        ] {
            assert_eq!(SessionAdapter::session_action(action), expected);
        }
    }

    #[test]
    fn lifecycle_pause_resume_flush_keep_loaded_media() {
        // Pause/resume/flush ride the shell command path, media stays put.
        let (mut adapter, _dir) = test_fixture::loaded_adapter();
        assert!(adapter.loaded());
        adapter.set_execution_held(true);
        adapter.pause();
        assert!(adapter.loaded());
        adapter.resume();
        assert!(adapter.loaded());
        adapter.set_execution_held(false);
        adapter.flush();
        assert!(adapter.loaded());
    }

    #[test]
    fn metrics_fold_hold_into_pause_display() {
        // Adapter-local hold forces the paused display without worker timing.
        let (mut adapter, _dir) = test_fixture::loaded_adapter();
        let tick = adapter.handle().tick_metrics();
        adapter.set_execution_held(true);
        assert!(adapter.metrics().paused);
        adapter.set_execution_held(false);
        assert_eq!(adapter.metrics().paused, tick.paused);
    }

    #[test]
    fn keyboard_repeat_suppression_and_controller_gating() {
        // Repeats collapse; the ungated path clears input after routing.
        use nerust_gui_application::KeyboardPort;
        use nerust_input_traits::Key;
        let (mut adapter, _dir) = test_fixture::loaded_adapter();
        let first = adapter.key_event(Key::KeyV, true, true);
        assert_eq!(adapter.key_event(Key::KeyV, true, true), None);
        adapter.key_event(Key::KeyV, false, true);
        assert_eq!(adapter.key_event(Key::KeyV, true, true), first);
        adapter.clear_input();
        assert_eq!(adapter.key_event(Key::KeyV, true, false), first);
    }

    #[test]
    fn frame_port_advances_and_serves_live_buffer() {
        // Advance pumps OSD; the live buffer and counters stay observable.
        use nerust_gui_application::FramePort;
        let (mut adapter, _dir) = test_fixture::loaded_adapter();
        assert!(adapter.frame().is_some());
        let counter = adapter.frame_counter();
        adapter.advance_frame();
        assert!(adapter.frame().is_some());
        assert_eq!(adapter.handle().tick_metrics().frame_counter, counter);
    }

    #[test]
    fn loaded_core_exposes_profile_factories_and_chrome() {
        // Read-only probes ride the live core without pausing or pumping.
        let (adapter, _dir) = test_fixture::loaded_adapter();
        assert!(adapter.render_profile().is_some());
        assert_eq!(adapter.factories().len(), 1);
        assert_eq!(adapter.osd_state().generation(), 0);
        let rates = adapter.supported_sample_rates();
        assert_eq!(rates, adapter.supported_sample_rates());
        assert!(rates.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(
            adapter.chrome(),
            nerust_gui_shell::session::osd::OsdChrome::from(
                &adapter.settings().local.notifications
            )
        );
    }

    #[test]
    fn modal_queue_reports_empty_by_default() {
        // Fresh adapter: no batch queued, ack closes nothing.
        let (mut adapter, _dir) = test_fixture::loaded_adapter();
        assert_eq!(adapter.take_modal_batch(), None);
        adapter.modal_closed();
        assert_eq!(adapter.take_modal_batch(), None);
    }

    #[test]
    fn history_port_clears_recorded_traffic() {
        // Slot traffic bumps the revision; clearing drains the lines.
        use nerust_gui_application::{HistoryPort, SessionActionPort};
        use nerust_gui_shell::session::session_query::HistoryView;
        let (mut adapter, _dir) = test_fixture::loaded_adapter();
        let language = adapter.settings().shared.general.language;
        let mut cache = HistoryView::new();
        assert!(adapter.history_lines(&mut cache, language).is_empty());
        let revision = adapter.history_revision();
        assert!(
            adapter
                .execute(SessionAction::CreateSlot)
                .expect("slot creates")
        );
        assert!(adapter.history_revision() > revision);
        assert!(!adapter.history_lines(&mut cache, language).is_empty());
        adapter.clear_history();
        assert!(adapter.history_lines(&mut cache, language).is_empty());
    }

    #[test]
    fn debug_probes_serve_live_core_without_pausing() {
        // Layout/live/begin probes run against the running core unpaused.
        use nerust_gui_shell::debug::{DebugAction, DebugDrainInput};
        let (adapter, _dir) = test_fixture::loaded_adapter();
        let spaces = adapter.debug_spaces();
        let input = DebugDrainInput {
            space: None,
            mem_addr: 0,
            dis_addr: None,
            watch: None,
            reg_target: None,
            freeze: None,
            prev_dump_rows: &[],
            need_images: false,
        };
        let mut flight = adapter.debug_begin(&[DebugAction::Refresh], &input);
        let _ = flight.poll();
        let (name, _layout) = adapter.debugger_layout_begin();
        assert_eq!(
            name,
            spaces
                .first()
                .map(|space| space.name.to_string())
                .unwrap_or_default()
        );
        let _images = adapter.ppu_layout_begin();
        let mut live = adapter.ppu_live_begin(Some(adapter.media_revision()));
        let _ = live.poll();
    }

    #[test]
    fn settings_round_trip_reapplies_snapshot() {
        // Settings port clones the live snapshot and absorbs it back.
        use nerust_gui_application::SettingsPort;
        let (mut adapter, _dir) = test_fixture::loaded_adapter();
        let snapshot = adapter.settings().clone();
        adapter.apply_settings(snapshot).expect("settings reapply");
    }

    #[test]
    fn media_reload_bumps_revision_and_rejects_missing_rom() {
        // Held reload skips the resume edge; missing files map to strings.
        use nerust_gui_application::MediaPort;
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let (mut adapter, _dir) = test_fixture::loaded_adapter();
        let before = adapter.media_revision();
        let rom = std::env::temp_dir().join(format!(
            "nerust-desktop-reload-{}-{}.nes",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::write(&rom, test_fixture::test_ines()).expect("reload ROM writes");
        adapter.set_execution_held(true);
        adapter.load(&rom).expect("held reload succeeds");
        adapter.set_execution_held(false);
        assert_eq!(adapter.media_revision(), before + 1);
        assert!(adapter.loaded());
        std::fs::remove_file(&rom).ok();
        assert!(
            adapter
                .load(std::path::Path::new("/nonexistent/rom.nes"))
                .is_err()
        );
    }

    #[test]
    fn slot_guards_and_commands_route_through_shell() {
        // Unknown slot ids short-circuit; every command maps to the shell.
        use nerust_gui_application::SessionActionPort;
        let (mut adapter, _dir) = test_fixture::loaded_adapter();
        for action in [
            SessionAction::SelectSlot(999_999),
            SessionAction::SaveSlot(999_999),
            SessionAction::LoadSlot(999_999),
            SessionAction::DeleteSlot(999_999),
        ] {
            assert_eq!(adapter.execute(action), Ok(false));
        }
        assert!(
            adapter
                .execute(SessionAction::CreateSlot)
                .expect("slot creates")
        );
        let (slots, _) = adapter.menu_slots();
        let id = slots.first().expect("slot recorded").slot_id;
        assert!(adapter.execute(SessionAction::SelectSlot(id)).is_ok());
        assert!(adapter.execute(SessionAction::SaveSlot(id)).is_ok());
        assert!(adapter.execute(SessionAction::LoadSlot(id)).is_ok());
        assert!(adapter.execute(SessionAction::NextSlot).is_ok());
        assert!(adapter.execute(SessionAction::PreviousSlot).is_ok());
        assert!(adapter.execute(SessionAction::SaveState).is_ok());
        assert!(adapter.execute(SessionAction::LoadState).is_ok());
        assert!(adapter.execute(SessionAction::DeleteSlot(id)).is_ok());
        assert!(adapter.execute(SessionAction::Reset).expect("reset runs"));
        assert!(adapter.execute(SessionAction::Unload).expect("unload runs"));
        // Unload drops media, not the core: unloaded reads false while the
        // live buffer and profile stay attached.
        assert!(!adapter.loaded());
        assert!(adapter.frame().is_some());
        assert!(adapter.render_profile().is_some());
    }
}
