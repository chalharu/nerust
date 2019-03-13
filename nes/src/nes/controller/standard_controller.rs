// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::Controller;
use crate::nes::OpenBusReadResult;

pub struct StandardController {
    buttons: [Buttons; 2],
    microphone: bool,
    index1: usize,
    index2: usize,
    strobe: bool,
}

bitflags! {
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
                    0
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
                    0
                },
                0x1F,
            ),
            _ => {
                error!("unhandled controller read at address: 0x{:04X}", address);
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
