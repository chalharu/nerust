use crate::descriptor::NesConsoleProfile;
use crate::load::NesLoadOptions;
use nerust_console::ConsoleMetrics;
use nerust_console::video::ConsoleVideo;
use nerust_gui_runtime::session::GuiSession;
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_session::core::WindowSize;
use nerust_input_nes::{
    NesInputState, StandardControllerSnapshot, decode_controller_state, encode_input_state,
};
use nerust_input_schema::DigitalInputEvent;
use nerust_persistence::model::StateSlotSummary;
use std::path::PathBuf;

#[derive(Debug)]
pub struct NesSession {
    session: GuiSession,
    input: NesInputState,
}

impl NesSession {
    pub fn new() -> Self {
        Self::from_gui_session(NesConsoleProfile.build_gui_session())
    }

    pub fn from_gui_session(session: GuiSession) -> Self {
        let mut result = Self {
            session,
            input: NesInputState::new(),
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
        let loaded = self.session.load(rom_path, data);
        if loaded {
            self.sync_input_from_session();
        }
        loaded
    }

    pub fn load_with_options(
        &mut self,
        rom_path: Option<PathBuf>,
        data: Vec<u8>,
        options: NesLoadOptions,
    ) -> bool {
        let loaded = self
            .session
            .load_with_options(rom_path, data, options.into_core_options());
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

    pub fn handle_controller_input(&mut self, event: DigitalInputEvent) {
        self.input.handle_input(event);
        self.apply_current_input_state();
    }

    pub fn clear_controller_input(&mut self) {
        let _ = self.input.clear_current_frame();
        self.apply_current_input_state();
    }

    fn current_controller_snapshot(&self) -> Option<StandardControllerSnapshot> {
        let bytes = self.session.current_controller_state().ok()?;
        match decode_controller_state(&bytes) {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                log::warn!("NES controller state decode failed: {error}");
                None
            }
        }
    }

    fn apply_current_input_state(&mut self) {
        let bytes = match encode_input_state(self.input.current_frame()) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("NES input state encode failed: {error}");
                return;
            }
        };
        self.session.apply_input_state(bytes);
    }

    fn sync_input_from_session(&mut self) {
        if let Some(snapshot) = self.current_controller_snapshot() {
            self.input.sync_from_snapshot(snapshot);
        } else {
            self.input = NesInputState::new();
        }
    }
}

impl Default for NesSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::NesSession;
    use crate::load::{NesLoadOptions, NesMmc3IrqVariant};
    use nerust_gui_runtime::session::GuiSession;
    use nerust_gui_session::core::SessionCore;
    use nerust_input_nes::{
        Buttons, NES_ATTACHMENT_PLAYER_ONE, NES_CONTROL_A, StandardControllerSnapshot,
        decode_controller_state,
    };
    use nerust_input_schema::DigitalInputEvent;
    use nerust_screen_buffer::screen_buffer::ScreenBuffer;
    use nerust_sound_traits::{MixerInput, Sound};

    #[derive(Default)]
    struct TestSpeaker;

    impl Sound for TestSpeaker {
        fn start(&mut self) {}

        fn pause(&mut self) {}
    }

    impl MixerInput for TestSpeaker {
        fn push(&mut self, _data: f32) {}
    }

    fn test_session() -> NesSession {
        NesSession::from_gui_session(GuiSession::from_session_core(SessionCore::from_console(
            nerust_console::Console::new(TestSpeaker, ScreenBuffer::new_nes_gpu_default()),
        )))
    }

    #[test]
    fn nes_session_flushes_digital_input_into_controller_state() {
        let mut session = test_session();

        session.handle_controller_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_A,
        ));

        let snapshot = decode_controller_state(
            &session
                .session
                .current_controller_state()
                .expect("controller state should export"),
        )
        .expect("controller state should decode");
        assert_eq!(
            snapshot,
            StandardControllerSnapshot {
                buttons: [Buttons::A, Buttons::empty()],
                microphone: false,
                index1: 0,
                index2: 0,
                strobe: false,
            }
        );
    }

    #[test]
    fn nes_load_options_flow_into_session_load() {
        let mut session = test_session();
        let mut rom = vec![
            0x4E, 0x45, 0x53, 0x1A, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        rom.resize(16 + 0x8000 + 0x2000, 0);

        assert!(session.load_with_options(
            None,
            rom,
            NesLoadOptions {
                mmc3_irq_variant: Some(NesMmc3IrqVariant::Sharp),
            },
        ));
    }
}
