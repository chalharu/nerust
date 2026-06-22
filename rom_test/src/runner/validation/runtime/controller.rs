use super::ValidationRuntime;
use crate::events::{ButtonCode, ControllerPad, PadState};
use crate::harness::apply_button_state;
use nerust_nes_core::input_types::Buttons;

impl ValidationRuntime {
    pub(in crate::runner::validation) fn apply_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    ) {
        let buttons = Buttons::from(button);
        match pad {
            ControllerPad::Pad1 => {
                self.pad1 = apply_button_state(self.pad1, buttons, state);
            }
            ControllerPad::Pad2 => {
                self.pad2 = apply_button_state(self.pad2, buttons, state);
            }
        }
        self.cell
            .store(self.pad1.bits(), self.pad2.bits(), self.mic);
    }

    pub(in crate::runner::validation) fn set_microphone(&mut self, state: PadState) {
        self.mic = matches!(state, PadState::Pressed);
        self.cell
            .store(self.pad1.bits(), self.pad2.bits(), self.mic);
    }
}
