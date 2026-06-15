use super::ValidationRuntime;
use crate::events::{ButtonCode, ControllerPad, PadState};
use crate::harness::apply_button_state;
use nerust_input_nes::frame::Buttons;

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
        self.cell.store(&[self.pad1.bits(), self.pad2.bits()]);
    }

    pub(in crate::runner::validation) fn set_microphone(&mut self, _state: PadState) {
        // NesPadDevice does not support microphone; ignored.
    }
}
