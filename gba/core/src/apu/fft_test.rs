//! FFT pitch/frequency tests for the GBA APU (test-only).
//!
//! Hand-rolled radix-2 FFT (Hann window, DC removal), ported from
//! `nes/core/src/apu/fft_test.rs` so `gba-core` gains no new dependency.
//! Samples come straight off the 32.768kHz native grid (`grid_buffer`
//! left channel): bin-exact resolution with no resampler smear. Test
//! tones sit exactly on FFT bins (512Hz = bin 128, 256Hz = bin 64 at
//! N = 8192), so leakage is minimal and the ±1.5-bin tolerance is generous.

use std::f32::consts::PI;

use super::{GbaApu, MIX_RATE};

pub(crate) const FFT_SAMPLE_COUNT: usize = 8192;
pub(crate) const GRID_RATE_HZ: f32 = MIX_RATE as f32;

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

/// Tick until `samples` grid samples exist; return the left channel.
pub(crate) fn capture_grid_mono(apu: &mut GbaApu, samples: usize) -> Vec<f32> {
    while apu.grid_buffer().len() < samples {
        apu.tick();
    }
    apu.grid_buffer()
        .iter()
        .take(samples)
        .map(|sample| sample.0)
        .collect()
}

/// Master on, all four PSG voices on L+R at full volume, PSG x4 gain, no
/// FIFO routing: a clean bus for single-voice pitch tests.
fn solo_bus(apu: &mut GbaApu) {
    apu.write_soundcnt_x(0x80);
    apu.write(0x04000080, 0xFFFF);
    apu.write_soundcnt_hi(0x0003);
}

#[test]
fn square_ch1_sounds_at_register_pitch() {
    let mut apu = GbaApu::new();
    solo_bus(&mut apu);
    // Sweep off (pace 0 decodes to the off-code 8, no shift).
    apu.write_sound1cnt_lo(0x00);
    // Duty 50%, length held (gate off), envelope frozen at 15.
    apu.write_sound1cnt_hi(0xF080);
    // freq 0x700 + trigger: 16.78MHz / (128 * (2048 - 0x700)) = 512Hz.
    apu.write_sound1cnt_x(0x8700);
    assert!(apu.sq1.core.active);

    let samples = capture_grid_mono(&mut apu, FFT_SAMPLE_COUNT);
    let dominant = dominant_frequency(&samples, GRID_RATE_HZ);
    let tolerance = dominant_frequency_tolerance(GRID_RATE_HZ, FFT_SAMPLE_COUNT);
    assert!(
        (dominant - 512.0).abs() <= tolerance,
        "square ch1 should sound 512Hz, got {dominant} (tol {tolerance})"
    );
}

#[test]
fn square_ch1_sweep_glides_upward_then_kills() {
    let mut apu = GbaApu::new();
    solo_bus(&mut apu);
    // Sweep pace 7 / increment / shift 2: each calculation adds a quarter
    // of the shadow, every 7th sequencer sweep tick (~55ms). From 0x300
    // the trigger-time calculation jumps straight to 0x3C0 (~120Hz),
    // periodic steps climb (~155Hz, ~239Hz), and the increment-mode
    // one-step-ahead overflow check kills the voice mid-glide (~160ms).
    apu.write_sound1cnt_lo(0x72);
    apu.write_sound1cnt_hi(0xF080);
    apu.write_sound1cnt_x(0x8300);
    assert!(apu.sq1.core.active);

    // 2048-sample windows (62.5ms each, 16Hz bins): the early window is
    // dominated by the ~120Hz start, the later one by the ~239Hz step.
    while apu.grid_buffer().len() < 5120 {
        apu.tick();
    }
    let early: Vec<f32> = apu.grid_buffer()[0..2048]
        .iter()
        .map(|sample| sample.0)
        .collect();
    let late: Vec<f32> = apu.grid_buffer()[3072..5120]
        .iter()
        .map(|sample| sample.0)
        .collect();
    let early_pitch = dominant_frequency(&early, GRID_RATE_HZ);
    let late_pitch = dominant_frequency(&late, GRID_RATE_HZ);
    assert!(
        late_pitch - early_pitch > 50.0,
        "sweep should glide up: early={early_pitch}, late={late_pitch}"
    );
    while apu.grid_buffer().len() < 6144 {
        apu.tick();
    }
    assert!(
        !apu.sq1.core.active,
        "sweep overflow should have killed the voice"
    );
}

#[test]
fn square_ch2_sounds_at_register_pitch() {
    let mut apu = GbaApu::new();
    solo_bus(&mut apu);
    // Duty 50%, length held (gate off), envelope frozen at 15.
    apu.write_sound2cnt_lo(0xF080);
    // freq 0x700 + trigger: 16.78MHz / (128 * (2048 - 0x700)) = 512Hz.
    apu.write_sound2cnt_hi(0x8700);
    assert!(apu.sq2.core.active);

    let samples = capture_grid_mono(&mut apu, FFT_SAMPLE_COUNT);
    let dominant = dominant_frequency(&samples, GRID_RATE_HZ);
    let tolerance = dominant_frequency_tolerance(GRID_RATE_HZ, FFT_SAMPLE_COUNT);
    assert!(
        (dominant - 512.0).abs() <= tolerance,
        "square ch2 should sound 512Hz, got {dominant} (tol {tolerance})"
    );
}

#[test]
fn wave_ch3_replays_table_at_register_rate() {
    fn sine_nibble(index: usize) -> u8 {
        (8.0 + 7.0 * (2.0 * PI * index as f32 / 32.0).sin()).round() as u8
    }

    let mut apu = GbaApu::new();
    solo_bus(&mut apu);
    // 32-digit sine table into BOTH banks (the CPU sees the non-playing
    // bank, so load each bank while the other one plays). Each wave RAM
    // byte holds two digits, even digit in the high nibble.
    for bank_bit in [0x0040u16, 0x0000] {
        apu.write_sound3cnt_lo(0x0080 | bank_bit);
        for word in 0..8u32 {
            let mut value = 0u16;
            for b in 0..2 {
                let digit = (2 * word + b) as usize;
                let byte = (u16::from(sine_nibble(2 * digit)) << 4)
                    | u16::from(sine_nibble(2 * digit + 1));
                value |= byte << (b * 8);
            }
            apu.wave_write(0x90 + word * 2, value);
        }
    }
    apu.write_sound3cnt_lo(0x0080);
    // Length 256 held (gate off), volume 100%.
    apu.write_sound3cnt_hi(0x2000);
    // rate 0x700 + trigger: 16.78MHz / (256 * (2048 - 0x700)) = 256Hz.
    apu.write_sound3cnt_x(0x8700);
    assert!(apu.wave.active);

    let samples = capture_grid_mono(&mut apu, FFT_SAMPLE_COUNT);
    let dominant = dominant_frequency(&samples, GRID_RATE_HZ);
    let tolerance = dominant_frequency_tolerance(GRID_RATE_HZ, FFT_SAMPLE_COUNT);
    assert!(
        (dominant - 256.0).abs() <= tolerance,
        "wave ch3 should replay 256Hz, got {dominant} (tol {tolerance})"
    );
    // A clean table replay keeps harmonics small: the octave-up peak
    // must stay well below the fundamental.
    let spectrum = power_spectrum(&samples);
    let fundamental = peak_power_near_frequency(&spectrum, GRID_RATE_HZ, 256.0, 2);
    let octave = peak_power_near_frequency(&spectrum, GRID_RATE_HZ, 512.0, 2);
    assert!(
        octave < fundamental / 8.0,
        "wave octave too hot: fund={fundamental}, octave={octave}"
    );
}

#[test]
fn noise_ch4_is_broadband_not_tonal() {
    let mut apu = GbaApu::new();
    solo_bus(&mut apu);
    // Volume 15, envelope off.
    apu.write_sound4cnt_lo(0xF000);
    // Trigger, length off, ratio 1, 15-bit, shift 3: the LFSR steps once
    // per grid sample, i.e. white at the native grid.
    apu.write_sound4cnt_hi(0x8031);
    assert!(apu.noise.core.active);

    let samples = capture_grid_mono(&mut apu, FFT_SAMPLE_COUNT);
    let spectrum = power_spectrum(&samples);
    let energy = average_band_power(&spectrum, GRID_RATE_HZ, 200.0, 6000.0);
    assert!(energy > 1e-9, "noise ch4 must sound, energy={energy}");
    let flatness = spectral_flatness(&spectrum, GRID_RATE_HZ, 200.0, 6000.0);
    assert!(
        flatness > 0.2,
        "noise ch4 must be broadband, flatness={flatness}"
    );
}
