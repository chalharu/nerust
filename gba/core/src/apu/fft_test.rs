//! FFT pitch/frequency tests for the GBA APU (test-only).
//!
//! Spectrum math lives in `nerust_sound_spectrum` (dev-dependency, so
//! production binaries never link it). Samples come straight off the
//! 32.768kHz native grid (`grid_buffer` left channel): bin-exact
//! resolution with no resampler smear. Test tones sit exactly on FFT
//! bins (512Hz = bin 128, 256Hz = bin 64 at N = 8192), so leakage is
//! minimal and the ±1.5-bin tolerance is generous.

use std::f32::consts::PI;

use super::{GbaApu, MIX_RATE};

pub(crate) const FFT_SAMPLE_COUNT: usize = 8192;
pub(crate) const GRID_RATE_HZ: f32 = MIX_RATE as f32;

pub(crate) use nerust_sound_spectrum::{
    average_band_power, dominant_frequency, dominant_frequency_tolerance,
    peak_power_near_frequency, power_spectrum, spectral_flatness,
};

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

fn sine_nibble(index: usize) -> u8 {
    (8.0 + 7.0 * (2.0 * PI * index as f32 / 32.0).sin()).round() as u8
}

/// 32-digit sine table into BOTH wave banks (the CPU sees the non-playing
/// bank, so load each bank while the other one plays). Each wave RAM byte
/// holds two digits, even digit in the high nibble.
fn load_sine_wave_banks(apu: &mut GbaApu) {
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
fn square_ch2_all_duties_shape_harmonics() {
    for duty in 0..4u16 {
        let mut apu = GbaApu::new();
        solo_bus(&mut apu);
        apu.write_sound2cnt_lo(0xF000 | duty << 6);
        apu.write_sound2cnt_hi(0x8700);
        assert!(apu.sq2.core.active);

        let samples = capture_grid_mono(&mut apu, FFT_SAMPLE_COUNT);
        let dominant = dominant_frequency(&samples, GRID_RATE_HZ);
        let tolerance = dominant_frequency_tolerance(GRID_RATE_HZ, FFT_SAMPLE_COUNT);
        assert!(
            (dominant - 512.0).abs() <= tolerance,
            "duty {duty} should sound 512Hz, got {dominant}"
        );
        let spectrum = power_spectrum(&samples);
        let fundamental = peak_power_near_frequency(&spectrum, GRID_RATE_HZ, 512.0, 2);
        let second = peak_power_near_frequency(&spectrum, GRID_RATE_HZ, 1024.0, 2);
        match duty {
            // 50%: half-wave symmetry cancels even harmonics.
            2 => assert!(
                second < fundamental / 16.0,
                "duty 2 should suppress the 2nd harmonic: fund={fundamental}, 2nd={second}"
            ),
            // 25%: strong 2nd harmonic (~half the fundamental power).
            1 => assert!(
                second > fundamental / 4.0,
                "duty 1 should carry a 2nd harmonic: fund={fundamental}, 2nd={second}"
            ),
            _ => {}
        }
    }
}

#[test]
fn wave_ch3_replays_table_at_register_rate() {
    let mut apu = GbaApu::new();
    solo_bus(&mut apu);
    load_sine_wave_banks(&mut apu);
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
fn wave_ch3_volume_codes_scale_output() {
    let mut powers = [0.0f32; 4];
    for code in 0..4u16 {
        let mut apu = GbaApu::new();
        solo_bus(&mut apu);
        load_sine_wave_banks(&mut apu);
        apu.write_sound3cnt_lo(0x0080);
        // Volume code: 0 mute, 1 full, 2 half, 3 quarter.
        apu.write_sound3cnt_hi(code << 13);
        apu.write_sound3cnt_x(0x8700);
        assert!(apu.wave.active);

        let samples = capture_grid_mono(&mut apu, 2048);
        let spectrum = power_spectrum(&samples);
        powers[code as usize] = average_band_power(&spectrum, GRID_RATE_HZ, 200.0, 320.0);
    }
    assert!(
        powers[1] > powers[2] && powers[2] > powers[3] && powers[3] > powers[0],
        "volume codes must scale 100% > 50% > 25% > mute, got {powers:?}"
    );
    assert!(
        powers[0] < 1e-12,
        "volume code 0 must silence the channel, got {}",
        powers[0]
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

#[test]
fn noise_ch4_7bit_mode_is_tonal() {
    let mut apu = GbaApu::new();
    solo_bus(&mut apu);
    // Volume 15, envelope off.
    apu.write_sound4cnt_lo(0xF000);
    // Trigger, length off, ratio 1, 7-bit LFSR, shift 3: one LFSR step
    // per grid sample, repeating every 127 samples (~258Hz metallic comb
    // — the “other” noise voice games use for cymbals and explosions).
    apu.write_sound4cnt_hi(0x8039);
    assert!(apu.noise.core.active);

    let samples = capture_grid_mono(&mut apu, FFT_SAMPLE_COUNT);
    let spectrum = power_spectrum(&samples);
    let peak = spectrum.iter().skip(1).copied().fold(0.0, f32::max);
    let average = average_band_power(&spectrum, GRID_RATE_HZ, 16.0, 16_380.0);
    assert!(
        peak / average > 15.0,
        "7-bit noise must show tonal peaks, peak={peak}, avg={average}"
    );
    let flatness = spectral_flatness(&spectrum, GRID_RATE_HZ, 200.0, 6000.0);
    assert!(
        flatness < 0.15,
        "7-bit noise must not be flat, flatness={flatness}"
    );
}

#[test]
fn direct_sound_fifo_path_replays_dac_pitch() {
    let mut apu = GbaApu::new();
    apu.write_soundcnt_x(0x80);
    // No PSG enables, full master volumes; FIFO A to L+R at x2 gain.
    apu.write(0x04000080, 0x0077);
    apu.write_soundcnt_hi(0x0300);
    // Step the DAC latch through a 256Hz sine (128 grid samples/cycle):
    // the timer/DMA feed is bypassed, pinning the DAC → mixer → bias
    // path that every streamed sample travels.
    for i in 0..FFT_SAMPLE_COUNT {
        let phase = 2.0 * PI * (i % 128) as f32 / 128.0;
        apu.dac_a = (100.0 * phase.sin()).round() as i8;
        for _ in 0..512 {
            apu.tick();
        }
    }
    let samples: Vec<f32> = apu.grid_buffer().iter().map(|sample| sample.0).collect();
    assert_eq!(samples.len(), FFT_SAMPLE_COUNT);
    let dominant = dominant_frequency(&samples, GRID_RATE_HZ);
    let tolerance = dominant_frequency_tolerance(GRID_RATE_HZ, FFT_SAMPLE_COUNT);
    assert!(
        (dominant - 256.0).abs() <= tolerance,
        "FIFO DAC path should replay 256Hz, got {dominant} (tol {tolerance})"
    );
}

#[test]
fn square_high_pitch_has_no_audible_foldover() {
    // se_select's top note (0x7D6 ~3121Hz, duty 50%): naive per-sample
    // square synthesis folds harmonics 7/9/11/... back as inharmonic
    // aliases (10923Hz at -17dB, 4681Hz at -19dB, ...), heard as metallic
    // harshness next to band-limited emulators. The polyBLEP stage must
    // push every foldover below -40dB of the fundamental; the lone
    // near-Nyquist residual (10923Hz) gets -30dB: it sits 22dB under the
    // neighboring 3rd harmonic (auditory masking) where the GBA speaker
    // has already rolled off steeply.
    let mut apu = GbaApu::new();
    solo_bus(&mut apu);
    apu.write_sound1cnt_lo(0x00);
    // Duty 50%, length held (gate off), envelope frozen at 15.
    apu.write_sound1cnt_hi(0xF080);
    apu.write_sound1cnt_x(0x87D6);
    assert!(apu.sq1.core.active);
    let samples = capture_grid_mono(&mut apu, FFT_SAMPLE_COUNT);
    let spectrum = power_spectrum(&samples);
    let fundamental = peak_power_near_frequency(&spectrum, GRID_RATE_HZ, 3121.0, 2);
    for alias in [1560.0, 4681.0, 6242.0, 7802.0, 10923.0, 12483.0, 14043.0] {
        let power = peak_power_near_frequency(&spectrum, GRID_RATE_HZ, alias, 2);
        let db = 10.0 * (power / fundamental).log10();
        let ceiling = if alias > 8000.0 { -30.0 } else { -40.0 };
        assert!(
            db < ceiling,
            "foldover at {alias}Hz must stay inaudible, got {db:.1}dB"
        );
    }
}
