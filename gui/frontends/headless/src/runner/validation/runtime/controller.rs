use super::ValidationRuntime;
use crate::events::{ControllerPad, PadState};

impl ValidationRuntime {
    pub(in crate::runner::validation) fn apply_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: String,
        state: PadState,
    ) -> Result<(), crate::error::RomTestError> {
        // The console picks the published state up in render_frame via
        // EmuInput. A button the pad lacks is a silent no-op (no such
        // hardware); field failures stay loud inside `set_button`.
        self.system
            .set_button(pad.index(), &button, matches!(state, PadState::Pressed))
    }
}
