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
    loader: Box<dyn RomLoader>,
    held: bool,
    pressed_keys: HashSet<wakui_core::Key>,
    audio: Arc<AudioBackendRegistry>,
    osd: nerust_gui_shell::session::osd::OsdManager,
}

impl SessionAdapter {
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
        (
            space
                .map(|space| space.name.to_string())
                .unwrap_or_default(),
            self.session
                .debug_drain_begin(&[nerust_gui_shell::debug::DebugAction::Refresh], &input),
        )
    }

    pub(crate) fn key(code: &panechord::KeyCode) -> Option<wakui_core::Key> {
        use serde::Deserialize;
        let mut name = String::new();
        for (index, character) in code.as_str().chars().enumerate() {
            if index > 0 && character.is_ascii_uppercase() {
                name.push('_');
            }
            name.push(character.to_ascii_lowercase());
        }
        wakui_core::Key::deserialize(
            serde::de::value::StrDeserializer::<serde::de::value::Error>::new(&name),
        )
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
    type Key = wakui_core::Key;
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
        if action == SessionAction::Unload {
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
            ("KeyV", wakui_core::Key::KeyV),
            ("F11", wakui_core::Key::F11),
            ("IntlYen", wakui_core::Key::IntlYen),
            ("MetaLeft", wakui_core::Key::SuperLeft),
            ("NumpadEnter", wakui_core::Key::NumpadEnter),
        ] {
            assert_eq!(
                SessionAdapter::key(&panechord::KeyCode::from_name(name)),
                Some(expected)
            );
        }
        assert_eq!(SessionAdapter::key(&panechord::KeyCode::Unidentified), None);
    }
}
