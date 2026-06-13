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

pub(crate) fn power_spectrum(samples: &[f32]) -> Vec<f32> {
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

    spectrum
        .iter()
        .take(samples.len() / 2)
        .map(|value| value.magnitude_squared())
        .collect()
}

pub(crate) fn dominant_frequency(samples: &[f32], sample_rate: f32) -> f32 {
    let spectrum = power_spectrum(samples);

    let mut best_bin = 1_usize;
    let mut best_magnitude = 0.0_f32;
    for (index, magnitude) in spectrum.iter().copied().enumerate().skip(1) {
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

pub(crate) fn peak_power_near_frequency(
    spectrum: &[f32],
    sample_rate: f32,
    frequency: f32,
    search_radius_bins: usize,
) -> f32 {
    let (start, end) = band_bounds(
        spectrum.len(),
        sample_rate,
        frequency.max(0.0),
        frequency.max(0.0),
    );
    let start = start.saturating_sub(search_radius_bins).max(1);
    let end = end
        .saturating_add(search_radius_bins)
        .min(spectrum.len().saturating_sub(1));
    spectrum[start..=end].iter().copied().fold(0.0, f32::max)
}

pub(crate) fn average_band_power(
    spectrum: &[f32],
    sample_rate: f32,
    start_frequency: f32,
    end_frequency: f32,
) -> f32 {
    let (start, end) = band_bounds(spectrum.len(), sample_rate, start_frequency, end_frequency);
    let band = &spectrum[start..=end];
    band.iter().copied().sum::<f32>() / band.len() as f32
}

pub(crate) fn spectral_flatness(
    spectrum: &[f32],
    sample_rate: f32,
    start_frequency: f32,
    end_frequency: f32,
) -> f32 {
    let (start, end) = band_bounds(spectrum.len(), sample_rate, start_frequency, end_frequency);
    let band = &spectrum[start..=end];
    let arithmetic_mean = band.iter().copied().sum::<f32>() / band.len() as f32;
    let geometric_mean = (band
        .iter()
        .copied()
        .map(|value| value.max(f32::MIN_POSITIVE).ln())
        .sum::<f32>()
        / band.len() as f32)
        .exp();
    geometric_mean / arithmetic_mean.max(f32::MIN_POSITIVE)
}

fn band_bounds(
    spectrum_len: usize,
    sample_rate: f32,
    start_frequency: f32,
    end_frequency: f32,
) -> (usize, usize) {
    assert!(spectrum_len > 1);
    assert!(end_frequency >= start_frequency);

    let max_bin = spectrum_len.saturating_sub(1).max(1);
    let sample_count = spectrum_len * 2;
    let start = frequency_bin(start_frequency, sample_rate, sample_count).clamp(1, max_bin);
    let end = frequency_bin(end_frequency, sample_rate, sample_count).clamp(start, max_bin);
    (start, end)
}

fn frequency_bin(frequency: f32, sample_rate: f32, sample_count: usize) -> usize {
    ((frequency * sample_count as f32 / sample_rate).round() as usize).min(sample_count / 2)
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
