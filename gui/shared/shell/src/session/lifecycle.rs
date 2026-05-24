use crate::descriptor::NesConsoleProfile;
use crate::load::NesLoadOptions;
use crate::session::NesSession;
use crate::settings::{current_or_default, effective_load_options};
use nerust_console::ConsoleMetrics;
use nerust_console::video::ConsoleVideo;
use nerust_gui_runtime::session::GuiSession;
use nerust_gui_runtime::settings::DesktopSettingsManager;
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_session::core::WindowSize;
use nerust_input_nes::input::NesInputState;
use nerust_persistence::model::StateSlotSummary;
use std::path::PathBuf;

impl NesSession {
    pub fn new(settings: DesktopSettingsManager) -> Self {
        Self::from_gui_session(
            NesConsoleProfile.build_gui_session(settings.clone()),
            settings,
        )
    }

    pub fn from_gui_session(session: GuiSession, settings: DesktopSettingsManager) -> Self {
        let mut result = Self {
            session,
            input: NesInputState::new(),
            settings,
        };
        result.sync_input_from_session();
        result
    }

    pub fn video(&self) -> &ConsoleVideo {
        self.session.video()
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.session.with_frame_buffer(f)
    }

    pub fn window_size(&self) -> WindowSize {
        self.session.window_size()
    }

    pub fn metrics(&self) -> ConsoleMetrics {
        self.session.metrics()
    }

    pub fn window_title(&self) -> String {
        self.session.window_title()
    }

    pub fn paused(&self) -> bool {
        self.session.paused()
    }

    pub fn loaded(&self) -> bool {
        self.session.loaded()
    }

    pub fn can_pause(&self) -> bool {
        self.session.can_pause()
    }

    pub fn can_resume(&self) -> bool {
        self.session.can_resume()
    }

    pub fn slots(&self) -> &[StateSlotSummary] {
        self.session.slots()
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.session.active_slot_id()
    }

    pub fn resume(&mut self) {
        self.session.resume();
    }

    pub fn load(&mut self, rom_path: Option<PathBuf>, data: Vec<u8>) -> bool {
        self.load_with_options(rom_path, data, NesLoadOptions::default())
    }

    pub fn load_with_options(
        &mut self,
        rom_path: Option<PathBuf>,
        data: Vec<u8>,
        options: NesLoadOptions,
    ) -> bool {
        let settings = current_or_default(&self.settings);
        let loaded = self.session.load_with_options(
            rom_path,
            data,
            effective_load_options(&settings, options).into_core_options(),
        );
        if loaded {
            self.sync_input_from_session();
        }
        loaded
    }

    pub fn unload(&mut self) -> bool {
        let unloaded = self.session.unload();
        if unloaded {
            self.sync_input_from_session();
        }
        unloaded
    }

    pub fn flush_before_exit(&mut self) {
        self.session.flush_before_exit();
    }

    pub fn run_command(&mut self, command: SessionCommand) -> SessionCommandOutcome {
        let outcome = self.session.run_command(command);
        if outcome.executed
            && matches!(
                command,
                SessionCommand::LoadActiveSlot | SessionCommand::LoadSlot(_)
            )
        {
            self.sync_input_from_session();
        }
        outcome
    }
}
