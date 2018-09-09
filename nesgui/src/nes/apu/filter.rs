// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::f64;

pub trait Filter {
    fn step(&mut self, data: f32) -> f32;
    fn chain<F: Filter>(self, filter: F) -> ChaindFilter<Self, F>
    where
        Self: Sized,
    {
        ChaindFilter::new(self, filter)
    }
}

pub(crate) struct PassFilter {
    b0: f32,
    b1: f32,
    a1: f32,
    prev_data: f32,
    prev_result: f32,
}

impl PassFilter {
    pub fn get_highpass_filter(sample_rate: f64, cutoff_freq: f32) -> Self {
        let c = (sample_rate * f64::consts::FRAC_1_PI / f64::from(cutoff_freq)) as f32;
        let a0i = 1.0 / (1.0 + c);
        Self {
            b0: a0i,
            b1: a0i,
            a1: (1.0 - c) * a0i,
            prev_result: 0.0,
            prev_data: 0.0,
        }
    }
    pub fn get_lowpass_filter(sample_rate: f64, cutoff_freq: f32) -> Self {
        let c = (sample_rate * f64::consts::FRAC_1_PI / f64::from(cutoff_freq)) as f32;
        let a0i = 1.0 / (1.0 + c);
        Self {
            b0: c * a0i,
            b1: -c * a0i,
            a1: (1.0 - c) * a0i,
            prev_result: 0.0,
            prev_data: 0.0,
        }
    }
}

impl Filter for PassFilter {
    fn step(&mut self, data: f32) -> f32 {
        self.prev_result = self.b0 * data + self.b1 * self.prev_data - self.a1 * self.prev_result;
        self.prev_data = data;
        return self.prev_result;
    }
}

pub struct ChaindFilter<F1: Filter, F2: Filter> {
    filter1: F1,
    filter2: F2,
}

impl<F1: Filter, F2: Filter> ChaindFilter<F1, F2> {
    fn new(filter1: F1, filter2: F2) -> Self {
        Self { filter1, filter2 }
    }
}

impl<F1: Filter, F2: Filter> Filter for ChaindFilter<F1, F2> {
    fn step(&mut self, data: f32) -> f32 {
        self.filter2.step(self.filter1.step(data))
    }
}
