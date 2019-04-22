// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

const LENGTH_TABLE: [u8; 32] = [
    0x0A, 0xFE, 0x14, 0x02, 0x28, 0x04, 0x50, 0x06, 0xA0, 0x08, 0x3C, 0x0A, 0x0E, 0x0C, 0x1A, 0x0E,
    0x0C, 0x10, 0x18, 0x12, 0x30, 0x14, 0x60, 0x16, 0xC0, 0x18, 0x48, 0x1A, 0x10, 0x1C, 0x20, 0x1E,
];

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub(crate) struct LengthCounterDao {
    next_halt: bool,
    halt: bool,
    enabled: bool,
    next_value: u8,
    value: u8,
    prev_value: u8,
}

impl LengthCounterDao {
    pub fn new() -> Self {
        Self {
            next_halt: false,
            halt: false,
            enabled: false,
            next_value: 0,
            value: 0,
            prev_value: 0,
        }
    }

    pub fn set_halt(&mut self, halt: bool) {
        self.next_halt = halt;
    }

    pub fn set_load(&mut self, value: u8) {
        if self.enabled {
            self.next_value = LENGTH_TABLE[usize::from(value)];
            self.prev_value = self.value;
        }
    }

    pub fn reset(&mut self) {
        self.enabled = false;
        self.halt = false;
        self.next_halt = false;
        self.next_value = 0;
        self.value = 0;
        self.prev_value = 0;
    }

    pub fn soft_reset(&mut self) {
        self.enabled = false;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.value = 0;
        }
        self.enabled = enabled;
    }

    pub fn step_frame(&mut self) {
        if !self.halt && self.value > 0 {
            self.value -= 1;
        }
    }

    pub fn step(&mut self) {
        if self.next_value > 0 {
            if self.value == self.prev_value {
                self.value = self.next_value;
            }
            self.next_value = 0;
        }

        self.halt = self.next_halt;
    }

    pub fn get_value(self) -> u8 {
        self.value
    }

    pub fn get_halt(self) -> bool {
        self.halt
    }

    pub fn get_status(self) -> bool {
        self.value > 0
    }
}

pub(crate) trait HaveLengthCounterDao {
    fn length_counter_dao(&self) -> &LengthCounterDao;
    fn length_counter_dao_mut(&mut self) -> &mut LengthCounterDao;
}

pub(crate) trait LengthCounter: HaveLengthCounterDao {
    fn set_enabled(&mut self, enabled: bool) {
        self.length_counter_dao_mut().set_enabled(enabled)
    }

    fn step_length(&mut self) {
        self.length_counter_dao_mut().step_frame()
    }

    fn step_length_counter(&mut self) {
        self.length_counter_dao_mut().step()
    }

    fn get_value(&self) -> u8 {
        self.length_counter_dao().get_value()
    }

    fn get_halt(&self) -> bool {
        self.length_counter_dao().get_halt()
    }

    fn get_status(&self) -> bool {
        self.length_counter_dao().get_status()
    }
}

impl<T: HaveLengthCounterDao> LengthCounter for T {}

pub(crate) trait HaveLengthCounter {
    type LengthCounter: LengthCounter;
    fn length_counter(&self) -> &Self::LengthCounter;
    fn length_counter_mut(&mut self) -> &mut Self::LengthCounter;
}
