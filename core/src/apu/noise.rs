// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::envelope::*;
use super::length_counter::*;
use super::timer::*;

// NTSC
// https://wiki.nesdev.com/w/index.php/APU_Noise
const NOISE_TABLE: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone)]
pub(crate) struct Noise {
    mode: bool,
    shift_register: u16,

    envelope: EnvelopeDao,
    length_counter: LengthCounterDao,
    timer: TimerDao,
}

impl HaveLengthCounterDao for Noise {
    fn length_counter_dao(&self) -> &LengthCounterDao {
        &self.length_counter
    }
    fn length_counter_dao_mut(&mut self) -> &mut LengthCounterDao {
        &mut self.length_counter
    }
}

impl HaveEnvelopeDao for Noise {
    fn envelope_dao(&self) -> &EnvelopeDao {
        &self.envelope
    }
    fn envelope_dao_mut(&mut self) -> &mut EnvelopeDao {
        &mut self.envelope
    }
}

impl HaveLengthCounter for Noise {
    type LengthCounter = Self;
    fn length_counter(&self) -> &Self::LengthCounter {
        self
    }
    fn length_counter_mut(&mut self) -> &mut Self::LengthCounter {
        self
    }
}

impl HaveTimerDao for Noise {
    fn timer_dao(&self) -> &TimerDao {
        &self.timer
    }
    fn timer_dao_mut(&mut self) -> &mut TimerDao {
        &mut self.timer
    }
}

impl Noise {
    pub(crate) fn new() -> Self {
        Self {
            mode: false,
            shift_register: 1,
            envelope: EnvelopeDao::new(),
            length_counter: LengthCounterDao::new(),
            timer: TimerDao::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.length_counter.reset();
        self.envelope.reset();
        self.timer.reset();
        self.timer.set_period(NOISE_TABLE[0] - 1);
        self.mode = false;
        self.shift_register = 1;
    }

    pub(crate) fn write_control(&mut self, value: u8) {
        self.length_counter.set_halt((value & 0x20) != 0);
        self.envelope.set_enabled((value & 0x10) == 0);
        self.envelope.set_period(value & 0x0F);
    }

    pub(crate) fn write_period(&mut self, value: u8) {
        self.mode = (value & 0x80) != 0;
        self.timer
            .set_period(NOISE_TABLE[usize::from(value & 0x0F)] - 1);
    }

    pub(crate) fn write_length(&mut self, value: u8) {
        self.length_counter.set_load(value >> 3);
        self.envelope.restart();
    }

    pub(crate) fn step_timer(&mut self) {
        if self.timer.step_timer() {
            self.shift_register = (self.shift_register >> 1)
                | (((self.shift_register & 1)
                    ^ ((self.shift_register >> (if self.mode { 6 } else { 1 })) & 1))
                    << 14);
        }
    }

    pub(crate) fn output(&self) -> u8 {
        if (self.shift_register & 1) != 0 {
            0
        } else {
            Envelope::get_volume(self)
        }
    }
}
