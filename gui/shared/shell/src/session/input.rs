use crate::session::NesSession;
use nerust_input_nes::codec::{decode_input_state, encode_input_state};
use nerust_input_nes::topology::{
    NES_ATTACHMENT_PLAYER_ONE, NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT,
    NES_CONTROL_RIGHT, NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP,
};
use nerust_input_schema::{DigitalControlId, DigitalInputEvent, DigitalInputState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NesButton {
    A,
    B,
    Select,
    Start,
    Up,
    Down,
    Left,
    Right,
}

impl NesButton {
    fn control_id(self) -> DigitalControlId {
        match self {
            Self::A => NES_CONTROL_A,
            Self::B => NES_CONTROL_B,
            Self::Select => NES_CONTROL_SELECT,
            Self::Start => NES_CONTROL_START,
            Self::Up => NES_CONTROL_UP,
            Self::Down => NES_CONTROL_DOWN,
            Self::Left => NES_CONTROL_LEFT,
            Self::Right => NES_CONTROL_RIGHT,
        }
    }

    fn player_one_event(self, pressed: bool) -> DigitalInputEvent {
        DigitalInputEvent::new(
            NES_ATTACHMENT_PLAYER_ONE,
            self.control_id(),
            if pressed {
                DigitalInputState::Pressed
            } else {
                DigitalInputState::Released
            },
        )
    }
}

impl NesSession {
    pub fn handle_controller_input(&mut self, event: DigitalInputEvent) {
        self.input.handle_input(event);
        self.apply_current_input_state();
    }

    pub fn handle_player_one_button(&mut self, button: NesButton, pressed: bool) {
        self.handle_controller_input(button.player_one_event(pressed));
    }

    pub fn clear_controller_input(&mut self) {
        let _ = self.input.clear_current_frame();
        self.apply_current_input_state();
    }

    fn current_input_frame(&self) -> Option<nerust_input_nes::frame::NesInputFrame> {
        let bytes = self.session.current_input_state().ok()?;
        match decode_input_state(&bytes) {
            Ok(frame) => Some(frame),
            Err(error) => {
                log::warn!("NES input state decode failed: {error}");
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

    pub(super) fn sync_input_from_session(&mut self) {
        if let Some(frame) = self.current_input_frame() {
            self.input.sync_from_frame(frame);
        } else {
            self.input = Default::default();
        }
    }
}
