use super::ValidationRuntime;
use crate::events::{ButtonCode, ControllerPad, PadState};
use nerust_input_traits::AbstractKey;

impl ValidationRuntime {
    pub(in crate::runner::validation) fn apply_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    ) -> Result<(), crate::error::RomTestError> {
        // The console picks the published state up in render_frame via
        // EmuInput. A button the pad lacks is a silent no-op (no such
        // hardware); field failures stay loud inside `set_button`.
        self.system.set_button(
            pad_index(pad),
            button.abstract_key(),
            matches!(state, PadState::Pressed),
        )
    }

    pub(in crate::runner::validation) fn set_microphone(
        &mut self,
        state: PadState,
    ) -> Result<(), crate::error::RomTestError> {
        // The microphone is just another button (pad 2, keyed like the
        // rest through the slot profile group).
        self.system.set_button(
            pad_index(ControllerPad::Pad2),
            AbstractKey::Button3,
            matches!(state, PadState::Pressed),
        )
    }
}

fn pad_index(pad: ControllerPad) -> usize {
    match pad {
        ControllerPad::Pad1 => 0,
        ControllerPad::Pad2 => 1,
    }
}
