// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::nes::{MixerInput, MixerOutput};
use std::{f32, i16};

pub trait Filter {
    fn step(&mut self, data: f32) -> f32;
    fn chain<F: Filter>(self, filter: F) -> ChaindFilter<Self, F>
    where
        Self: Sized,
    {
        ChaindFilter::new(self, filter)
    }
}

pub(crate) struct IirFilter {
    b0: f32,
    b1: f32,
    a1: f32,
    prev_data: f32,
    prev_result: f32,
}

// 双一次変換を利用する
impl IirFilter {
    pub fn get_highpass_filter(sample_rate: f32, cutoff_freq: f32) -> Self {
        let t = 1.0 / sample_rate;
        let omega_c = 2.0 * f32::consts::PI * cutoff_freq;
        let c = (omega_c * t / 2.0).tan();

        let b0 = 1.0 / (1.0 + c);
        let b1 = -b0;
        let a1 = (1.0 - c) / (1.0 + c);

        Self {
            b0,
            b1,
            a1,
            prev_result: 0.0,
            prev_data: 0.0,
        }
    }
    pub fn get_lowpass_filter(sample_rate: f32, cutoff_freq: f32) -> Self {
        let t = 1.0 / sample_rate;
        let omega_c = 2.0 * f32::consts::PI * cutoff_freq;
        let c = (omega_c * t / 2.0).tan();

        let b0 = c / (1.0 + c);
        let b1 = b0;
        let a1 = (1.0 - c) / (1.0 + c);

        Self {
            b0,
            b1,
            a1,
            prev_result: 0.0,
            prev_data: 0.0,
        }
    }
}

impl Filter for IirFilter {
    fn step(&mut self, data: f32) -> f32 {
        self.prev_result = self.b0 * data + self.b1 * self.prev_data + self.a1 * self.prev_result;
        self.prev_data = data;
        self.prev_result
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

pub struct Mixer<F: Filter> {
    data: Box<[f32]>,
    filter: F,
    rate: f64,
    next_cycle: f64,
    cycle: f64,
    output_pos: usize,
    input_pos: usize,
}

impl<F: Filter> Mixer<F> {
    pub fn new(filter: F, buffer_width: usize, push_rate: usize, sample_rate: usize) -> Self {
        let data = vec![0.0; buffer_width.next_power_of_two()].into_boxed_slice();
        Self {
            filter,
            data,
            rate: push_rate as f64 / sample_rate as f64,
            cycle: 0.0,
            next_cycle: 0.0,
            output_pos: 0,
            input_pos: 0,
        }
    }
}

impl<F: Filter> MixerInput for Mixer<F> {
    // 0.0 ~ 1.0 => -1.0 ~ 1.0
    fn push(&mut self, data: f32) {
        self.cycle += 1.0;
        if self.cycle > self.next_cycle {
            self.data[self.input_pos] = data * 2.0 - 1.0;
            self.next_cycle += self.rate;
            self.input_pos = (self.input_pos + 1) & (self.data.len() - 1);
        }
    }
}

// 16bit data
impl<F: Filter> MixerOutput for Mixer<F> {}

impl<F: Filter> Iterator for Mixer<F> {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        let result =
            ((&mut self.filter).step(self.data[self.output_pos]) * i16::max_value() as f32) as i16;
        self.output_pos = (self.output_pos + 1) & (self.data.len() - 1);
        Some(result)
    }
}

pub(crate) type NesMixer = Mixer<ChaindFilter<ChaindFilter<IirFilter, IirFilter>, IirFilter>>;

impl NesMixer {
    pub fn nes_mixer(sample_rate: f32, bufer_width: usize, push_rate: usize) -> Self {
        Mixer::new(
            IirFilter::get_lowpass_filter(sample_rate, 14000.0)
                .chain(IirFilter::get_highpass_filter(sample_rate, 90.0))
                .chain(IirFilter::get_highpass_filter(sample_rate, 442.0)),
            bufer_width,
            push_rate,
            sample_rate as usize,
        )
    }
}
