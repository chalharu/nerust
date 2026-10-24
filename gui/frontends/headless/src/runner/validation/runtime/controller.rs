use super::ValidationRuntime;
use crate::events::{ControllerPad, PadState};

impl ValidationRuntime {
    pub(in crate::runner::validation) fn apply_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: String,
        state: PadState,
    ) -> Result<(), crate::error::RomTestError> {
        // The console picks the published state up in render_frame.
        // Unknown buttons are loud errors inside `set_button`.
        self.system
            .control()
            .set_button(pad.index(), &button, matches!(state, PadState::Pressed))
    }
}
