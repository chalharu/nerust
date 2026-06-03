// Copyright (c) 2018 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

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
                self.controller.set_pad1(self.pad1);
            }
            ControllerPad::Pad2 => {
                self.pad2 = apply_button_state(self.pad2, buttons, state);
                self.controller.set_pad2(self.pad2);
            }
        }
    }

    pub(in crate::runner::validation) fn set_microphone(&mut self, state: PadState) {
        self.controller
            .set_microphone(matches!(state, PadState::Pressed));
    }
}
