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

/// Multiplier cap on the internal oversampling rate relative to device rate.
/// Applied in `SimpleDownSampler::new(u32, u32)` to limit resampler filter load.
const OVERSAMPLE_FACTOR: u32 = 4;

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
    source_rate: u32,
}

impl SimpleDownSampler {
    /// Create with automatic OVERSAMPLE_FACTOR cap.
    ///
    /// `source_rate` is capped to at most `OVERSAMPLE_FACTOR × destination_rate`
    /// to limit the resampler's internal filter load. The effective source rate
    /// can be retrieved via [`source_rate()`](Self::source_rate).
    pub fn new(source_rate: u32, destination_rate: u32) -> Self {
        let capped = source_rate
            .min(destination_rate.saturating_mul(OVERSAMPLE_FACTOR))
            .max(destination_rate);
        Self::new_raw(capped as f64, destination_rate as f64)
    }

    /// Create without cap (raw floating-point rates).
    ///
    /// * `source_rate` – sample rate of the input stream (Hz).
    /// * `destination_rate` – target output sample rate (Hz).
    pub fn new_raw(source_rate: f64, destination_rate: f64) -> Self {
        let rate = source_rate / destination_rate;
        let filter = IirFilter::get_lowpass_filter(source_rate as f32, destination_rate as f32);
        Self {
            rate,
            cycle: 0.0,
            next_cycle: 0.0,
            filter,
            source_rate: source_rate as u32,
        }
    }

    /// The effective input sample rate of this resampler (Hz).
    pub fn source_rate(&self) -> u32 {
        self.source_rate
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
        let mut resampler = SimpleDownSampler::new_raw(src, dst);

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
        let mut resampler = SimpleDownSampler::new_raw(src, dst);

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
        let mut resampler = SimpleDownSampler::new_raw(rate, rate);

        let outputs: usize = (0..1000).filter_map(|_| resampler.step(0.0)).count();
        assert_eq!(outputs, 1000, "1:1 ratio should emit one output per input");
    }
}
