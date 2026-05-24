use crate::descriptor::{
    FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
    NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT, NES_CONTROL_RIGHT,
    NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP,
};
use nerust_console::{ControllerInputs, NesInputFrame};
use nerust_gui_runtime::session::GuiSession;
use nerust_input_schema::{AttachmentId, DigitalControlId, DigitalInputEvent, DigitalInputState};

#[derive(Debug, Default)]
pub struct NesInputAdapter {
    held: NesInputFrame,
    dirty_player_one: bool,
    dirty_player_two: bool,
    dirty_microphone: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NesDigitalTarget {
    PadOne(ControllerInputs),
    PadTwo(ControllerInputs),
    Microphone,
}

impl NesInputAdapter {
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
                self.held.microphone = matches!(event.state, DigitalInputState::Pressed);
                self.dirty_microphone = true;
            }
        }
    }

    pub fn flush_to_session(&self, session: &mut GuiSession) {
        let mut frame = session.current_nes_input_frame();
        if self.dirty_player_one {
            frame.player_one = self.held.player_one;
        }
        if self.dirty_player_two {
            frame.player_two = self.held.player_two;
        }
        if self.dirty_microphone {
            frame.microphone = self.held.microphone;
        }
        session.apply_nes_input_frame(frame);
    }

    pub fn clear(&mut self, session: &mut GuiSession) {
        if self.dirty_player_one {
            self.held.player_one = ControllerInputs::empty();
        }
        if self.dirty_player_two {
            self.held.player_two = ControllerInputs::empty();
        }
        if self.dirty_microphone {
            self.held.microphone = false;
        }
        self.flush_to_session(session);
    }

    pub fn current_frame(&self) -> NesInputFrame {
        self.held
    }

    pub fn sync_from_session(&mut self, session: &GuiSession) {
        self.held = session.current_nes_input_frame();
        self.dirty_player_one = false;
        self.dirty_player_two = false;
        self.dirty_microphone = false;
    }

    fn digital_target(
        attachment: AttachmentId,
        control: DigitalControlId,
    ) -> Option<NesDigitalTarget> {
        let button = match control {
            NES_CONTROL_A => Some(ControllerInputs::A),
            NES_CONTROL_B => Some(ControllerInputs::B),
            NES_CONTROL_SELECT => Some(ControllerInputs::SELECT),
            NES_CONTROL_START => Some(ControllerInputs::START),
            NES_CONTROL_UP => Some(ControllerInputs::UP),
            NES_CONTROL_DOWN => Some(ControllerInputs::DOWN),
            NES_CONTROL_LEFT => Some(ControllerInputs::LEFT),
            NES_CONTROL_RIGHT => Some(ControllerInputs::RIGHT),
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

    fn apply_button_state(
        current: ControllerInputs,
        flag: ControllerInputs,
        state: DigitalInputState,
    ) -> ControllerInputs {
        match state {
            DigitalInputState::Pressed => current | flag,
            DigitalInputState::Released => current & !flag,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NesInputAdapter;
    use crate::descriptor::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
        NES_CONTROL_A, NES_CONTROL_RIGHT, NES_CONTROL_SELECT,
    };
    use nerust_console::{ControllerInputs, NesInputFrame};
    use nerust_gui_runtime::session::GuiSession;
    use nerust_gui_session::core::SessionCore;
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

    fn test_session() -> GuiSession {
        GuiSession::from_session_core(SessionCore::from_console(nerust_console::Console::new(
            TestSpeaker,
            ScreenBuffer::new_nes_gpu_default(),
        )))
    }

    #[test]
    fn nes_input_adapter_maps_player_one_buttons_into_nes_input_frame() {
        let mut adapter = NesInputAdapter::new();

        adapter.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_A,
        ));
        adapter.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_RIGHT,
        ));
        adapter.handle_input(DigitalInputEvent::released(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_A,
        ));

        assert_eq!(
            adapter.current_frame(),
            NesInputFrame {
                player_one: ControllerInputs::RIGHT,
                ..NesInputFrame::default()
            }
        );
    }

    #[test]
    fn nes_input_adapter_maps_player_two_microphone_without_accepting_select() {
        let mut adapter = NesInputAdapter::new();

        adapter.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_TWO,
            FAMICOM_P2_CONTROL_MICROPHONE,
        ));
        adapter.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_TWO,
            NES_CONTROL_SELECT,
        ));

        assert_eq!(
            adapter.current_frame(),
            NesInputFrame {
                microphone: true,
                ..NesInputFrame::default()
            }
        );
    }

    #[test]
    fn nes_input_adapter_preserves_unowned_session_inputs_when_flushing() {
        let mut session = test_session();
        let mut adapter = NesInputAdapter::new();

        session.apply_nes_input_frame(NesInputFrame {
            player_two: ControllerInputs::LEFT,
            microphone: true,
            ..NesInputFrame::default()
        });
        adapter.sync_from_session(&session);
        adapter.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_A,
        ));
        adapter.flush_to_session(&mut session);

        assert_eq!(
            session.current_nes_input_frame(),
            NesInputFrame {
                player_one: ControllerInputs::A,
                player_two: ControllerInputs::LEFT,
                microphone: true,
            }
        );
    }
}
