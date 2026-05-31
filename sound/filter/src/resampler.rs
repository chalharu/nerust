// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{Filter, IirFilter};

/// Trait for single-sample audio resamplers.
pub trait Resampler {
    /// Advance the resampler by one source sample.
    ///
    /// Returns `Some(sample)` when a downsampled output sample is ready,
    /// or `None` when the current source sample is consumed by the filter
    /// but no output sample is produced yet.
    fn step(&mut self, data: f32) -> Option<f32>;
}

/// Downsampling resampler with an IIR low-pass anti-alias filter.
///
/// Converts a high-rate source stream to a lower-rate destination stream by
/// applying an IIR low-pass filter at the destination Nyquist frequency and
/// then dropping samples proportionally to the ratio.
#[derive(Debug, Clone)]
pub struct SimpleDownSampler {
    rate: f64,
    cycle: f64,
    next_cycle: f64,
    filter: IirFilter,
}

impl SimpleDownSampler {
    /// Create a new `SimpleDownSampler`.
    ///
    /// * `source_rate` – sample rate of the input stream (Hz).
    /// * `destination_rate` – target output sample rate (Hz).
    pub fn new(source_rate: f64, destination_rate: f64) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsampler_produces_fewer_outputs_than_inputs() {
        // Source at 4× destination: expect roughly 1 output per 4 inputs.
        let src = 192_000.0_f64;
        let dst = 48_000.0_f64;
        let mut resampler = SimpleDownSampler::new(src, dst);

        let total_inputs = 4096_usize;
        let outputs: Vec<f32> = (0..total_inputs)
            .filter_map(|_| resampler.step(0.5))
            .collect();

        // Allow ±1 sample tolerance for rounding
        let expected = total_inputs / 4;
        assert!(
            outputs.len().abs_diff(expected) <= 1,
            "expected ~{expected} outputs, got {}",
            outputs.len()
        );
    }

    #[test]
    fn downsampler_passes_dc_signal() {
        // A constant signal should converge to itself after the filter settles.
        let src = 96_000.0_f64;
        let dst = 48_000.0_f64;
        let mut resampler = SimpleDownSampler::new(src, dst);

        // Warm up the filter
        for _ in 0..256 {
            let _ = resampler.step(1.0);
        }
        // Collect 64 samples after warm-up
        let outputs: Vec<f32> = (0..512).filter_map(|_| resampler.step(1.0)).collect();

        assert!(!outputs.is_empty(), "should have produced some outputs");
        for &s in &outputs {
            assert!(
                (s - 1.0).abs() < 0.01,
                "DC signal should pass through: got {s}"
            );
        }
    }

    #[test]
    fn downsampler_1to1_ratio_emits_every_sample() {
        // When source == destination rate, every input should yield an output.
        let rate = 48_000.0_f64;
        let mut resampler = SimpleDownSampler::new(rate, rate);

        let outputs: usize = (0..1000).filter_map(|_| resampler.step(0.0)).count();
        assert_eq!(outputs, 1000, "1:1 ratio should emit one output per input");
    }
}
