// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::LENGTH_TABLE;

// NTSC
// https://wiki.nesdev.com/w/index.php/APU_Noise
const NOISE_TABLE: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

pub(crate) struct Noise {
    pub enabled: bool,
    mode: bool,
    shift_register: u16,
    length_enabled: bool,
    pub length_value: u8,
    timer_period: u16,
    timer_value: u16,

    envelope_enabled: bool,
    envelope_loop: bool,
    envelope_start: bool,
    envelope_period: u8,
    envelope_value: u8,
    envelope_volume: u8,
    constant_volume: u8,
}

impl Noise {
    pub fn new() -> Self {
        Self {
            enabled: false,
            mode: false,
            shift_register: 1,
            length_enabled: false,
            length_value: 0,
            timer_period: 0,
            timer_value: 0,
            envelope_enabled: false,
            envelope_loop: false,
            envelope_start: false,
            envelope_period: 0,
            envelope_value: 0,
            envelope_volume: 0,
            constant_volume: 0,
        }
    }

    pub fn write_control(&mut self, value: u8) {
        self.length_enabled = ((value >> 5) & 1) == 0;
        self.envelope_loop = ((value >> 5) & 1) != 0;
        self.envelope_enabled = ((value >> 4) & 1) != 0;
        self.envelope_period = value & 0x0F;
        self.constant_volume = value & 0x0F;
        self.envelope_start = true;
    }

    pub fn write_period(&mut self, value: u8) {
        self.mode = (value & 0x80) == 0;
        self.timer_period = NOISE_TABLE[usize::from(value & 0x0F)];
    }

    pub fn write_length(&mut self, value: u8) {
        self.length_value = LENGTH_TABLE[usize::from(value & 0x0F)];
        self.envelope_start = true;
    }

    pub fn step_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;

            self.shift_register = (self.shift_register >> 1)
                | ((self.shift_register & 1
                    ^ (self.shift_register >> (if self.mode { 6 } else { 1 })) & 1)
                    << 14);
        } else {
            self.timer_value -= 1;
        }
    }

    pub fn step_envelope(&mut self) {
        if self.envelope_start {
            self.envelope_volume = 15;
            self.envelope_value = self.envelope_period;
            self.envelope_start = false;
        } else if self.envelope_value > 0 {
            self.envelope_value -= 1;
        } else {
            if self.envelope_volume > 0 {
                self.envelope_volume -= 1;
            } else if self.envelope_loop {
                self.envelope_volume = 15;
            }
            self.envelope_value = self.envelope_period;
        }
    }

    pub fn step_length(&mut self) {
        if self.length_enabled && self.length_value > 0 {
            self.length_value -= 1;
        }
    }

    pub fn output(&self) -> u8 {
        if !self.enabled || self.length_value == 0 || (self.shift_register & 1) != 0 {
            0
        } else if self.envelope_enabled {
            self.envelope_volume
        } else {
            self.constant_volume
        }
    }
}
