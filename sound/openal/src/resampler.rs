// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_soundfilter::{Filter, IirFilter};
use std::f32;

pub(crate) trait Resampler {
    fn step(&mut self, data: f32) -> Option<f32>;
}

// LPFで出力周波数以上を除去
#[derive(Debug)]
pub(crate) struct SimpleDownSampler {
    rate: f64,
    cycle: f64,
    next_cycle: f64,
    filter: IirFilter,
}

impl SimpleDownSampler {
    pub(crate) fn new(source_rate: f64, destination_rate: f64) -> Self {
        let rate = source_rate / destination_rate;
        let filter = IirFilter::get_lowpass_filter(source_rate as f32, destination_rate as f32);
        Self {
            rate,
            cycle: 0.0,
            next_cycle: 0.0,
            filter,
        }
    }
}

impl Resampler for SimpleDownSampler {
    fn step(&mut self, data: f32) -> Option<f32> {
        self.cycle += 1.0;
        let result = self.filter.step(data);
        if self.cycle > self.next_cycle {
            self.next_cycle += self.rate;
            Some(result)
        } else {
            None
        }
    }
}
