// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::Controller;
use crate::OpenBusReadResult;

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy)]
pub struct StandardController {
    buttons: [Buttons; 2],
    microphone: bool,
    index1: usize,
    index2: usize,
    strobe: bool,
}

bitflags::bitflags! {
    #[derive(
        serde_derive::Serialize,
        serde_derive::Deserialize,
        Debug,
        Clone,
        Copy,
    )]
    pub struct Buttons: u8 {
        const A =      0b0000_0001;
        const B =      0b0000_0010;
        const SELECT = 0b0000_0100;
        const START =  0b0000_1000;
        const UP =     0b0001_0000;
        const DOWN =   0b0010_0000;
        const LEFT =   0b0100_0000;
        const RIGHT =  0b1000_0000;
    }
}

impl StandardController {
    pub fn new() -> Self {
        Self {
            buttons: [Buttons::empty(); 2],
            microphone: false,
            index1: 0,
            index2: 0,
            strobe: false,
        }
    }

    pub fn reset(&mut self) {
        self.buttons = [Buttons::empty(); 2];
    }

    pub fn set_pad1(&mut self, buttons: Buttons) {
        self.buttons[0] = buttons;
    }

    pub fn set_pad2(&mut self, buttons: Buttons) {
        self.buttons[1] = buttons;
    }

    pub fn set_pad(&mut self, buttons: [Buttons; 2]) {
        self.buttons = buttons;
    }
}

impl Default for StandardController {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller for StandardController {
    fn read(&mut self, address: usize) -> OpenBusReadResult {
        match address {
            0 => OpenBusReadResult::new(
                if self.index1 < 8 {
                    let result = self.buttons[0].bits() >> self.index1;
                    if !self.strobe {
                        self.index1 += 1;
                    }
                    result & 1
                } else {
                    1
                } | (if self.microphone { 0x04 } else { 0 }),
                7,
            ),
            1 => OpenBusReadResult::new(
                if self.index2 < 8 {
                    let result = self.buttons[1].bits() >> self.index2;
                    if !self.strobe {
                        self.index2 += 1;
                    }
                    result & 1
                } else {
                    1
                },
                0x1F,
            ),
            _ => {
                log::error!("unhandled controller read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0)
            }
        }
    }

    fn write(&mut self, value: u8) {
        self.strobe = value & 1 == 1;
        if self.strobe {
            self.index1 = 0;
            self.index2 = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Buttons, StandardController};
    use crate::controller::Controller;

    #[test]
    fn standard_controller_returns_one_after_eight_bits() {
        let mut controller = StandardController::new();
        controller.set_pad1(Buttons::A);

        controller.write(1);
        controller.write(0);

        for _ in 0..8 {
            let _ = controller.read(0);
        }

        assert_eq!(controller.read(0).data & 1, 1);
        assert_eq!(controller.read(0).data & 1, 1);
    }
}
