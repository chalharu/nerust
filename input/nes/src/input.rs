pub mod persisted;

use crate::frame::{Buttons, NesInputFrame};
use crate::topology::{
    FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
    NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT, NES_CONTROL_RIGHT,
    NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP,
};
use nerust_input_schema::{AttachmentId, DigitalControlId, DigitalInputEvent, DigitalInputState};

#[derive(Debug, Default)]
pub struct NesInputState {
    held: NesInputFrame,
    dirty_player_one: bool,
    dirty_player_two: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NesDigitalTarget {
    PadOne(Buttons),
    PadTwo(Buttons),
    Microphone,
}

impl NesInputState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_input(&mut self, event: DigitalInputEvent) {
        let Some(target) = Self::digital_target(event.attachment, event.control) else {
            return;
        };
        match target {
            NesDigitalTarget::PadOne(flag) => {
                self.held.player_one =
                    Self::apply_button_state(self.held.player_one, flag, event.state);
                self.dirty_player_one = true;
            }
            NesDigitalTarget::PadTwo(flag) => {
                self.held.player_two =
                    Self::apply_button_state(self.held.player_two, flag, event.state);
                self.dirty_player_two = true;
            }
            NesDigitalTarget::Microphone => {
                // ファミコン P2 マイク。P2 の SELECT ビット位置 (0x04) として扱う。
                let flag = Buttons::SELECT;
                self.held.player_two =
                    Self::apply_button_state(self.held.player_two, flag, event.state);
                self.dirty_player_two = true;
            }
        }
    }

    pub fn clear_current_frame(&mut self) -> NesInputFrame {
        if self.dirty_player_one {
            self.held.player_one = Buttons::empty();
        }
        if self.dirty_player_two {
            self.held.player_two = Buttons::empty();
        }
        self.held
    }

    pub fn sync_from_frame(&mut self, frame: NesInputFrame) {
        self.held = frame;
        self.dirty_player_one = false;
        self.dirty_player_two = false;
    }

    pub fn current_frame(&self) -> NesInputFrame {
        self.held
    }

    fn digital_target(
        attachment: AttachmentId,
        control: DigitalControlId,
    ) -> Option<NesDigitalTarget> {
        let button = match control {
            NES_CONTROL_A => Some(Buttons::A),
            NES_CONTROL_B => Some(Buttons::B),
            NES_CONTROL_SELECT => Some(Buttons::SELECT),
            NES_CONTROL_START => Some(Buttons::START),
            NES_CONTROL_UP => Some(Buttons::UP),
            NES_CONTROL_DOWN => Some(Buttons::DOWN),
            NES_CONTROL_LEFT => Some(Buttons::LEFT),
            NES_CONTROL_RIGHT => Some(Buttons::RIGHT),
            _ => None,
        };

        match attachment {
            NES_ATTACHMENT_PLAYER_ONE => button.map(NesDigitalTarget::PadOne),
            NES_ATTACHMENT_PLAYER_TWO => {
                if control == FAMICOM_P2_CONTROL_MICROPHONE {
                    Some(NesDigitalTarget::Microphone)
                } else {
                    button.and_then(|flag| match control {
                        NES_CONTROL_SELECT | NES_CONTROL_START => None,
                        _ => Some(NesDigitalTarget::PadTwo(flag)),
                    })
                }
            }
            _ => None,
        }
    }

    fn apply_button_state(current: Buttons, flag: Buttons, state: DigitalInputState) -> Buttons {
        match state {
            DigitalInputState::Pressed => current | flag,
            DigitalInputState::Released => current & !flag,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NesInputState;
    use crate::frame::{Buttons, NesInputFrame};
    use crate::topology::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
        NES_CONTROL_A, NES_CONTROL_RIGHT,
    };
    use nerust_input_schema::DigitalInputEvent;

    #[test]
    fn nes_input_state_maps_player_one_buttons() {
        let mut input = NesInputState::new();

        input.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_A,
        ));
        input.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_RIGHT,
        ));
        input.handle_input(DigitalInputEvent::released(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_A,
        ));

        assert_eq!(
            input.current_frame(),
            NesInputFrame {
                player_one: Buttons::RIGHT,
                ..NesInputFrame::default()
            }
        );
    }

    #[test]
    fn nes_input_state_preserves_synced_frame_fields() {
        let mut input = NesInputState::new();
        let base = NesInputFrame {
            player_one: Buttons::empty(),
            player_two: Buttons::LEFT,
            microphone: true,
        };
        input.sync_from_frame(base);
        input.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_A,
        ));

        assert_eq!(
            input.current_frame(),
            NesInputFrame {
                player_one: Buttons::A,
                player_two: Buttons::LEFT,
                microphone: true,
            }
        );
    }

    #[test]
    fn microphone_maps_to_player_two_select_bit() {
        let mut input = NesInputState::new();

        input.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_TWO,
            FAMICOM_P2_CONTROL_MICROPHONE,
        ));

        let frame = input.current_frame();
        assert!(
            frame.player_two.contains(Buttons::SELECT),
            "mic should set P2 SELECT bit"
        );
        assert!(!frame.microphone, "mic field is deprecated");
    }
}
