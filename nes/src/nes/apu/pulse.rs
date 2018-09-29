// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::LENGTH_TABLE;

const DUTY_TABLE: [[bool; 8]; 4] = [
    [false, true, false, false, false, false, false, false],
    [false, true, true, false, false, false, false, false],
    [false, true, true, true, true, false, false, false],
    [true, false, false, true, true, true, true, true],
];

pub(crate) struct Pulse {
    pub enabled: bool,
    is_first_channel: bool,
    length_enabled: bool,
    pub length_value: u8,
    timer_period: u16,
    timer_value: u16,
    duty_mode: u8,
    duty_value: u8,
    sweep_reload: bool,
    sweep_enabled: bool,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_period: u8,
    sweep_value: u8,
    envelope_enabled: bool,
    envelope_loop: bool,
    envelope_start: bool,
    envelope_period: u8,
    envelope_value: u8,
    envelope_volume: u8,
    constant_volume: u8,
}

impl Pulse {
    pub fn new(is_first_channel: bool) -> Self {
        Self {
            enabled: false,
            is_first_channel,
            length_enabled: false,
            length_value: 0,
            timer_period: 0,
            timer_value: 0,
            duty_mode: 0,
            duty_value: 0,
            sweep_reload: false,
            sweep_enabled: false,
            sweep_negate: false,
            sweep_shift: 0,
            sweep_period: 0,
            sweep_value: 0,
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
        self.duty_mode = (value >> 6) & 3;
        self.length_enabled = ((value >> 5) & 1) == 0;
        self.envelope_loop = ((value >> 5) & 1) == 1;
        self.envelope_enabled = ((value >> 4) & 1) == 0;
        self.envelope_period = value & 0x0F;
        self.constant_volume = value & 0x0F;
        self.envelope_start = true;
    }

    pub fn write_sweep(&mut self, value: u8) {
        self.sweep_enabled = ((value >> 7) & 1) == 1;
        self.sweep_period = ((value >> 4) & 7) + 1;
        self.sweep_negate = ((value >> 3) & 1) == 1;
        self.sweep_shift = value & 7;
        self.sweep_reload = true;
    }

    pub fn write_timer_low(&mut self, value: u8) {
        self.timer_period = (self.timer_period & 0xFF00) | u16::from(value);
    }

    pub fn write_timer_high(&mut self, value: u8) {
        if self.enabled {
            self.length_value = LENGTH_TABLE[usize::from(value >> 3)];
        }
        self.timer_period = (self.timer_period & 0xFF) | (u16::from(value & 7) << 8);
        self.envelope_start = true;
        self.duty_value = 0;
    }

    pub fn step_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            self.duty_value = (self.duty_value + 1) & 7;
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

    pub fn step_sweep(&mut self) {
        if self.sweep_reload {
            if self.sweep_enabled && self.sweep_value == 0 {
                self.sweep();
            }
            self.sweep_value = self.sweep_period;
            self.sweep_reload = false;
        } else if self.sweep_value > 0 {
            self.sweep_value -= 1;
        } else {
            if self.sweep_enabled {
                self.sweep();
            }
            self.sweep_value = self.sweep_period;
        }
    }

    pub fn step_length(&mut self) {
        if self.length_enabled && self.length_value > 0 {
            self.length_value -= 1;
        }
    }

    pub fn sweep(&mut self) {
        let delta = self.timer_period >> self.sweep_shift;
        if self.sweep_negate {
            self.timer_period -= delta;
            if self.is_first_channel {
                self.timer_period -= 1;
            }
        } else {
            self.timer_period += delta;
        }
    }

    pub fn output(&self) -> u8 {
        if !self.enabled
            || self.length_value == 0
            || DUTY_TABLE[usize::from(self.duty_mode)][usize::from(self.duty_value)]
            || self.timer_period < 8
            || self.timer_period > 0x7FF
        {
            0
        } else if self.envelope_enabled {
            self.envelope_volume
        } else {
            self.constant_volume
        }
    }
}
