// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::envelope::*;
use super::length_counter::*;

const DUTY_TABLE: [[bool; 8]; 4] = [
    [false, true, false, false, false, false, false, false],
    [false, true, true, false, false, false, false, false],
    [false, true, true, true, true, false, false, false],
    [true, false, false, true, true, true, true, true],
];

pub(crate) struct Pulse {
    is_first_channel: bool,
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
    envelope: EnvelopeDao,
    length_counter: LengthCounterDao,
}

impl HaveLengthCounterDao for Pulse {
    fn length_counter_dao(&self) -> &LengthCounterDao {
        &self.length_counter
    }
    fn length_counter_dao_mut(&mut self) -> &mut LengthCounterDao {
        &mut self.length_counter
    }
}

impl HaveEnvelopeDao for Pulse {
    fn envelope_dao(&self) -> &EnvelopeDao {
        &self.envelope
    }
    fn envelope_dao_mut(&mut self) -> &mut EnvelopeDao {
        &mut self.envelope
    }
}

impl HaveLengthCounter for Pulse {
    type LengthCounter = Self;
    fn length_counter(&self) -> &Self::LengthCounter {
        self
    }
    fn length_counter_mut(&mut self) -> &mut Self::LengthCounter {
        self
    }
}

impl Pulse {
    pub fn new(is_first_channel: bool) -> Self {
        Self {
            is_first_channel,
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
            envelope: EnvelopeDao::new(),
            length_counter: LengthCounterDao::new(),
        }
    }

    pub fn reset(&mut self) {
        self.length_counter.reset();
        self.envelope.reset();

        /*
        _duty = 0;
		_dutyPos = 0;

		_realPeriod = 0;

		_sweepEnabled = false;
		_sweepPeriod = 0;
		_sweepNegate = false;
		_sweepShift = 0;
		_reloadSweep = false;
		_sweepDivider = 0;
		_sweepTargetPeriod = 0;
		UpdateTargetPeriod();
        */
    }

    pub fn write_control(&mut self, value: u8) {
        self.length_counter.set_halt((value & 0x20) != 0);
        self.envelope.set_enabled((value & 0x10) == 0);
        self.envelope.set_period(value & 0x0F);
        self.duty_mode = (value >> 6) & 3;
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
        self.length_counter.set_load(value >> 3);
        self.timer_period = (self.timer_period & 0xFF) | (u16::from(value & 7) << 8);
        self.duty_value = 0;
        self.envelope.restart();
    }

    pub fn step_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            self.duty_value = (self.duty_value + 1) & 7;
        } else {
            self.timer_value -= 1;
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
        if (self.timer_period < 8 || (!self.sweep_negate && self.timer_period > 0x7FF))
            && !DUTY_TABLE[usize::from(self.duty_mode)][usize::from(self.duty_value)]
        {
            0
        } else {
            Envelope::get_volume(self)
        }
    }
}
