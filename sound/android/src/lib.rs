//! Android-native audio backend built on CPAL / AAudio.
//!
//! On Android API 26+, CPAL selects AAudio as the audio backend automatically.
//! This crate wraps the CPAL output stream with the `Sound + MixerInput` traits
//! used throughout the nerust audio pipeline.
//!
//! The CPAL dependency and the public [`AndroidSound`] type are only compiled on
//! `target_os = "android"`.  On other targets the crate is intentionally empty
//! so it does not pull in platform-specific audio system libraries during
//! desktop development builds.
//!
//! # Lifecycle
//!
//! * Call [`AndroidSound::with_gain`] once at startup.  Backend creation is
//!   **fallible** – do not silently fall back if it fails; propagate the error
//!   to the caller so the problem is surfaced.
//! * Call [`Sound::start`] / [`Sound::pause`] to mirror the app lifecycle
//!   (foreground / background).  These map directly to `stream.play()` /
//!   `stream.pause()` on the underlying CPAL stream.
//! * Feed samples via [`MixerInput::push`]; the NES APU calls this at the rate
//!   returned by [`MixerInput::sample_rate`].

#[cfg(target_os = "android")]
pub mod android;

/// Tests that exercise the source-rate clamping formula and the filter/resampler
/// pipeline shared with the Android backend.  These run on all targets so they
/// can be validated during desktop development without a real Android device.
#[cfg(test)]
mod tests {
    use nerust_soundfilter::resampler::{Resampler, SimpleDownSampler};
    use nerust_soundfilter::{Filter, NesFilter};

    /// The multiplier cap – must match `OVERSAMPLE_FACTOR` in `android.rs`.
    const OVERSAMPLE_FACTOR: u32 = 4;

    /// Source sample rate is capped at OVERSAMPLE_FACTOR × playback rate.
    #[test]
    fn source_rate_capped_at_oversample_factor() {
        let playback: u32 = 48_000;
        let output_rate: u32 = 1_789_773; // NES CPU clock rate
        let source_sample_rate = output_rate
            .min(playback.saturating_mul(OVERSAMPLE_FACTOR))
            .max(playback);
        assert_eq!(source_sample_rate, playback * OVERSAMPLE_FACTOR);
    }

    /// When output_rate ≤ playback_rate, source rate clamps to playback rate.
    #[test]
    fn source_rate_never_below_playback_rate() {
        let playback: u32 = 48_000;
        let output_rate: u32 = 8_000;
        let source_sample_rate = output_rate
            .min(playback.saturating_mul(OVERSAMPLE_FACTOR))
            .max(playback);
        assert_eq!(source_sample_rate, playback);
    }

    /// The push path maps [0, 1] → [-1, 1].  A 0.5 mid-point signal should
    /// converge to ~0 after the NES high-pass filters settle.
    #[test]
    fn push_maps_midpoint_to_near_zero() {
        let playback: f32 = 48_000.0;
        let mut filter = NesFilter::new(playback);
        let mut resampler = SimpleDownSampler::new(192_000.0, f64::from(playback));

        let mut collected = Vec::new();
        for _ in 0..4096 {
            if let Some(r) = resampler.step(0.5) {
                collected.push(filter.step((r * 2.0 - 1.0) * 1.0));
            }
        }

        // After settling, DC bias should be removed by the high-pass stages.
        if let Some(&last) = collected.last() {
            assert!(
                last.abs() < 0.05,
                "mid-point input should converge to ~0 after filter settling, got {last}"
            );
        }
    }
}
