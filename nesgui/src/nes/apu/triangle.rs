// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::LENGTH_TABLE;

const TRIANGLE_TABLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

pub(crate) struct Triangle {
    pub enabled: bool,
    length_enabled: bool,
    pub length_value: u8,
    timer_period: u16,
    timer_value: u16,
    duty_value: u8,
    counter_period: u8,
    counter_value: u8,
    counter_reload: bool,
}

impl Triangle {
    pub fn new() -> Self {
        Self {
            enabled: false,
            length_enabled: false,
            length_value: 0,
            timer_period: 0,
            timer_value: 0,
            duty_value: 0,
            counter_reload: false,
            counter_period: 0,
            counter_value: 0,
        }
    }

    pub fn write_control(&mut self, value: u8) {
        self.length_enabled = ((value >> 7) & 1) == 0;
        self.counter_period = value & 0x7F;
    }

    pub fn write_timer_low(&mut self, value: u8) {
        self.timer_period = (self.timer_period & 0xFF00) | u16::from(value);
    }

    pub fn write_timer_high(&mut self, value: u8) {
        if self.enabled {
            self.length_value = LENGTH_TABLE[usize::from(value >> 3)];
            self.timer_period = (self.timer_period & 0xFF) | (u16::from(value & 7) << 8);
            self.timer_value = self.timer_period;
            self.counter_reload = true;
        }
    }

    pub fn step_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            if self.length_value > 0 && self.counter_value > 0 {
                self.duty_value = (self.duty_value + 1) & 0x1F
            }
        } else {
            self.timer_value -= 1;
        }
    }

    pub fn step_length(&mut self) {
        if self.length_enabled && self.length_value > 0 {
            self.length_value -= 1;
        }
    }

    pub fn step_counter(&mut self) {
        if self.counter_reload {
            self.counter_value = self.counter_period;
        } else if self.counter_value > 0 {
            self.counter_value -= 1;
        }
        if self.length_enabled {
            self.counter_reload = false;
        }
    }

    pub fn output(&self) -> u8 {
        if !self.enabled || self.length_value == 0 || self.counter_value == 0 {
            0
        } else {
            TRIANGLE_TABLE[usize::from(self.duty_value)]
        }
    }
}
