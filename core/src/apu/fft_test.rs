// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::f32::consts::PI;

pub(crate) const CPU_CLOCK_HZ: f32 = 1_789_773.0;
pub(crate) const FFT_SAMPLE_COUNT: usize = 16_384;

#[derive(Clone, Copy, Debug, Default)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    fn magnitude_squared(self) -> f32 {
        self.re.mul_add(self.re, self.im * self.im)
    }
}

pub(crate) fn capture_samples<F>(samples: usize, mut next_sample: F) -> Vec<f32>
where
    F: FnMut() -> f32,
{
    let mut captured = Vec::with_capacity(samples);
    for _ in 0..samples {
        captured.push(next_sample());
    }
    captured
}

pub(crate) fn dominant_frequency(samples: &[f32], sample_rate: f32) -> f32 {
    assert!(samples.len() > 1);
    assert!(samples.len().is_power_of_two());

    let mean = samples.iter().copied().sum::<f32>() / samples.len() as f32;
    let last_index = (samples.len() - 1) as f32;
    let mut spectrum = Vec::with_capacity(samples.len());
    for (index, sample) in samples.iter().copied().enumerate() {
        let window = 0.5 - 0.5 * ((2.0 * PI * index as f32) / last_index).cos();
        spectrum.push(Complex {
            re: (sample - mean) * window,
            im: 0.0,
        });
    }

    fft(&mut spectrum);

    let mut best_bin = 1_usize;
    let mut best_magnitude = 0.0_f32;
    for (index, value) in spectrum.iter().enumerate().take(samples.len() / 2).skip(1) {
        let magnitude = value.magnitude_squared();
        if magnitude > best_magnitude {
            best_magnitude = magnitude;
            best_bin = index;
        }
    }

    best_bin as f32 * sample_rate / samples.len() as f32
}

pub(crate) fn dominant_frequency_tolerance(sample_rate: f32, sample_count: usize) -> f32 {
    1.5 * sample_rate / sample_count as f32
}

fn fft(values: &mut [Complex]) {
    assert!(values.len().is_power_of_two());

    let mut bit_reversed_index = 0_usize;
    for index in 1..values.len() {
        let mut bit = values.len() >> 1;
        while (bit_reversed_index & bit) != 0 {
            bit_reversed_index ^= bit;
            bit >>= 1;
        }
        bit_reversed_index ^= bit;
        if index < bit_reversed_index {
            values.swap(index, bit_reversed_index);
        }
    }

    let mut block_size = 2;
    while block_size <= values.len() {
        let half_block = block_size / 2;
        let angle = -2.0 * PI / block_size as f32;
        let twiddle_step = Complex {
            re: angle.cos(),
            im: angle.sin(),
        };

        let mut block_start = 0;
        while block_start < values.len() {
            let mut twiddle = Complex { re: 1.0, im: 0.0 };
            for offset in 0..half_block {
                let even = values[block_start + offset];
                let odd = values[block_start + offset + half_block];
                let rotated_odd = Complex {
                    re: odd.re.mul_add(twiddle.re, -(odd.im * twiddle.im)),
                    im: odd.re.mul_add(twiddle.im, odd.im * twiddle.re),
                };
                values[block_start + offset] = Complex {
                    re: even.re + rotated_odd.re,
                    im: even.im + rotated_odd.im,
                };
                values[block_start + offset + half_block] = Complex {
                    re: even.re - rotated_odd.re,
                    im: even.im - rotated_odd.im,
                };
                twiddle = Complex {
                    re: twiddle
                        .re
                        .mul_add(twiddle_step.re, -(twiddle.im * twiddle_step.im)),
                    im: twiddle
                        .re
                        .mul_add(twiddle_step.im, twiddle.im * twiddle_step.re),
                };
            }
            block_start += block_size;
        }

        block_size <<= 1;
    }
}
