// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::f32;

pub trait Resampler {
    fn step(&mut self, data: f32) -> Option<f32>;
}

pub(crate) struct SimpleResampler {
    rate: f64,
    cycle: f64,
    next_cycle: f64,
}

impl SimpleResampler {
    pub fn new(source_rate: f64, destination_rate: f64) -> Self {
        let rate = source_rate / destination_rate;
        Self {
            rate,
            cycle: 0.0,
            next_cycle: 0.0,
        }
    }
}

impl Resampler for SimpleResampler {
    fn step(&mut self, data: f32) -> Option<f32> {
        self.cycle += 1.0;
        if self.cycle > self.next_cycle {
            self.next_cycle += self.rate;
            Some(data)
        } else {
            None
        }
    }
}
