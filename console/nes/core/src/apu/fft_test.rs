//! NES APU test items; spectrum math lives in `nerust_sound_spectrum`
//! (dev-dependency, so production binaries never link it).

pub(crate) const CPU_CLOCK_HZ: f32 = 1_789_773.0;
pub(crate) const FFT_SAMPLE_COUNT: usize = 16_384;

pub(crate) use nerust_sound_spectrum::{
    average_band_power, capture_samples, dominant_frequency, dominant_frequency_tolerance,
    peak_power_near_frequency, power_spectrum, spectral_flatness,
};
