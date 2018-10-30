// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::length_counter::{HaveLengthCounter, LengthCounter};

pub(crate) struct EnvelopeDao {
    enabled: bool,
    volume: u8,
    start: bool,
    value: u8,
    period: u8,
}

impl EnvelopeDao {
    pub fn new() -> Self {
        Self {
            enabled: false,
            volume: 0,
            start: false,
            value: 0,
            period: 0,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_period(&mut self, value: u8) {
        self.period = value;
    }

    pub fn restart(&mut self) {
        self.start = true;
    }

    pub fn reset(&mut self) {
        self.enabled = false;
        self.start = false;
        self.volume = 0;
        self.value = 0;
        self.period = 0;
    }

    pub fn get_volume(&self) -> u8 {
        if self.enabled {
            self.volume
        } else {
            self.period
        }
    }

    pub fn step_frame(&mut self, loop_: bool) {
        if self.start {
            self.volume = 15;
            self.value = self.period;
            self.start = false;
        } else if self.value > 0 {
            self.value -= 1;
        } else {
            if self.volume > 0 {
                self.volume -= 1;
            } else if loop_ {
                self.volume = 15;
            }
            self.value = self.period;
        }
    }
}

pub(crate) trait HaveEnvelopeDao {
    fn envelope_dao(&self) -> &EnvelopeDao;
    fn envelope_dao_mut(&mut self) -> &mut EnvelopeDao;
}

pub(crate) trait Envelope: HaveEnvelopeDao + HaveLengthCounter {
    fn get_volume(&self) -> u8 {
        if self.length_counter().get_value() > 0 {
            self.envelope_dao().get_volume()
        } else {
            0
        }
    }

    fn step_frame(&mut self) {
        let l = self.length_counter().get_halt();
        self.envelope_dao_mut().step_frame(l)
    }
}

impl<T: HaveEnvelopeDao + HaveLengthCounter> Envelope for T {}

pub(crate) trait HaveEnvelope {
    type Envelope: Envelope;
    fn envelope(&self) -> &Self::Envelope;
    fn envelope_mut(&mut self) -> &mut Self::Envelope;
}
