// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub trait Controller {
    fn read(&mut self, _address: usize) -> u8;
    fn write(&mut self, _value: u8);
}

pub(crate) struct StandardController {
    buttons: [Buttons; 2],
    microphone: bool,
    index1: usize,
    index2: usize,
    strobe: bool,
}

bitflags! {
    pub struct Buttons: u8 {
        const A = 0b00000001;
        const B = 0b00000010;
        const SELECT = 0b00000100;
        const START = 0b00001000;
        const UP = 0b00010000;
        const DOWN = 0b00100000;
        const LEFT = 0b01000000;
        const RIGHT = 0b10000000;
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
    fn read(&mut self, address: usize) -> u8 {
        match address {
            0 => {
                (if self.index1 < 8 {
                    let result = self.buttons[0].bits() >> self.index1;
                    if !self.strobe {
                        self.index1 += 1;
                    }
                    result
                } else {
                    0
                }) | (if self.microphone { 0x04 } else { 0 })
            }
            1 => {
                if self.index2 < 8 {
                    let result = self.buttons[1].bits() >> self.index2;
                    if !self.strobe {
                        self.index2 += 1;
                    }
                    result
                } else {
                    0
                }
            }
            _ => {
                error!("unhandled controller read at address: 0x{:04X}", address);
                0
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
