//! Dynamic rate-control filter: the pipeline stage between EmuCore and
//! `AudioBackend`.
//!
//! Data flow: `EmuCore` produces nominal-rate samples → this filter
//! stretches them against the backend queue level → `AudioBackend` plays
//! them. Slow emulation speeds the pitch up slightly instead of gaping on
//! silence; ahead-of-time emulation slows down slightly instead of
//! flooding the queue (RetroArch-style dynamic rate control).
//!
//! Cores stay bit-exact nominal producers and backends stay dumb sinks:
//! all adaptation state lives here, owned by the session layer, so none
//! of it can leak into machine states.

use nerust_core_traits::audio::{AudioBackend, StereoSample};

/// Proportional controller holding the backend queue near half-full.
/// Output is the device-samples-per-core-sample ratio (1.0 = nominal).
#[derive(Debug, Clone)]
pub struct RateController {
    ratio: f32,
    target_fill: f32,
    max_adjust: f32,
    gain: f32,
    slew: f32,
}

impl Default for RateController {
    fn default() -> Self {
        Self {
            ratio: 1.0,
            target_fill: 0.5,
            max_adjust: 0.05,
            gain: 0.6,
            slew: 0.003,
        }
    }
}

impl RateController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current ratio (1.0 = nominal rate).
    pub fn ratio(&self) -> f32 {
        self.ratio
    }

    /// Back to nominal (queue contents no longer match the timeline:
    /// load/unload/reset, backend reconnect).
    pub fn reset(&mut self) {
        self.ratio = 1.0;
    }

    /// Feed one control step's queue occupancy; returns the updated ratio.
    /// Unknown backends (`capacity == 0`) pin the ratio at nominal.
    pub fn update(&mut self, buffered: u64, capacity: u64) -> f32 {
        if capacity == 0 {
            self.ratio = 1.0;
            return self.ratio;
        }
        let fill = (buffered as f32 / capacity as f32).clamp(0.0, 1.5);
        let target = (1.0 + self.gain * (self.target_fill - fill))
            .clamp(1.0 - self.max_adjust, 1.0 + self.max_adjust);
        let delta = (target - self.ratio).clamp(-self.slew, self.slew);
        self.ratio += delta;
        self.ratio
    }
}

/// Frame-granular dynamic resampler (linear interpolation with a live
/// ratio). Session-owned: one per emulation session, reset on lifecycle
/// transitions. At ratio exactly 1.0 the output is bit-identical to the
/// input (direct passthrough, no float rounding).
#[derive(Debug)]
pub struct DynamicRateFilter {
    controller: RateController,
    prev: StereoSample,
    frac: f64,
    since_update: u64,
}

impl DynamicRateFilter {
    pub fn new() -> Self {
        Self {
            controller: RateController::new(),
            prev: StereoSample::SILENCE,
            frac: 0.0,
            since_update: 0,
        }
    }

    /// Current stretch ratio (observability for tests/metrics).
    pub fn ratio(&self) -> f32 {
        self.controller.ratio()
    }

    pub fn reset(&mut self) {
        self.controller.reset();
        self.prev = StereoSample::SILENCE;
        self.frac = 0.0;
        self.since_update = 0;
    }

    /// Stretch one frame's nominal samples against the backend queue
    /// level and push the result. The control step runs about once per
    /// frame worth of pushed samples (`sample_rate / 60`); between steps
    /// the last ratio holds.
    pub fn push_frame(&mut self, samples: &[StereoSample], backend: &mut dyn AudioBackend) {
        let interval = (backend.sample_rate() / 60).max(1) as u64;
        let mut ratio = self.controller.ratio();
        for &sample in samples {
            self.since_update += 1;
            if self.since_update >= interval {
                self.since_update = 0;
                ratio = self
                    .controller
                    .update(backend.buffered(), backend.buffer_capacity());
            }
            if ratio == 1.0 {
                backend.push(sample);
            } else {
                self.frac += f64::from(ratio);
                while self.frac >= 1.0 {
                    self.frac -= 1.0;
                    let t = (1.0 - self.frac) as f32;
                    backend.push(StereoSample::new(
                        self.prev.left + (sample.left - self.prev.left) * t,
                        self.prev.right + (sample.right - self.prev.right) * t,
                    ));
                }
            }
            self.prev = sample;
        }
    }
}

impl Default for DynamicRateFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake backend with scripted occupancy and recorded output.
    struct Probe {
        buffered: u64,
        capacity: u64,
        pushed: Vec<StereoSample>,
    }

    impl Probe {
        fn new(buffered: u64, capacity: u64) -> Self {
            Self {
                buffered,
                capacity,
                pushed: Vec::new(),
            }
        }
    }

    impl AudioBackend for Probe {
        fn start(&mut self) {}
        fn pause(&mut self) {}
        fn push(&mut self, sample: StereoSample) {
            self.pushed.push(sample);
        }
        fn buffered(&self) -> u64 {
            self.buffered
        }
        fn buffer_capacity(&self) -> u64 {
            self.capacity
        }
    }

    fn ramp(n: usize) -> Vec<StereoSample> {
        (0..n)
            .map(|i| StereoSample::new(i as f32 / n as f32, -(i as f32) / n as f32))
            .collect()
    }

    #[test]
    fn unknown_backend_passes_through_bit_exact() {
        let mut filter = DynamicRateFilter::new();
        let mut backend = Probe::new(0, 0);
        let input = ramp(2000);
        filter.push_frame(&input, &mut backend);
        assert_eq!(backend.pushed, input);
        assert_eq!(filter.ratio(), 1.0);
    }

    #[test]
    fn starved_queue_expands_output() {
        let mut filter = DynamicRateFilter::new();
        let mut backend = Probe::new(0, 4800);
        let input = ramp(20_000);
        filter.push_frame(&input, &mut backend);
        assert!(
            backend.pushed.len() > input.len(),
            "starved queue must gain samples: {} vs {}",
            backend.pushed.len(),
            input.len()
        );
        assert!(filter.ratio() > 1.0);
    }

    #[test]
    fn flooded_queue_compresses_output() {
        let mut filter = DynamicRateFilter::new();
        let mut backend = Probe::new(4800, 4800);
        let input = ramp(20_000);
        filter.push_frame(&input, &mut backend);
        assert!(
            backend.pushed.len() < input.len(),
            "flooded queue must lose samples: {} vs {}",
            backend.pushed.len(),
            input.len()
        );
        assert!(filter.ratio() < 1.0);
    }

    #[test]
    fn half_full_queue_holds_steady() {
        let mut filter = DynamicRateFilter::new();
        let mut backend = Probe::new(2400, 4800);
        // Pre-warm away from the initial state, then hold at half fill.
        filter.push_frame(&ramp(20_000), &mut Probe::new(0, 4800));
        filter.reset();
        let input = ramp(20_000);
        filter.push_frame(&input, &mut backend);
        let drift = backend.pushed.len().abs_diff(input.len());
        assert!(
            drift * 100 < input.len(),
            "half-full queue must stay within 1%: drift={drift}"
        );
        assert!((filter.ratio() - 1.0).abs() < 0.01);
    }

    #[test]
    fn output_stays_finite_and_continuous() {
        let mut filter = DynamicRateFilter::new();
        let mut backend = Probe::new(0, 4800);
        let input = ramp(5000);
        filter.push_frame(&input, &mut backend);
        assert!(backend.pushed.iter().all(|s| s.left.is_finite()));
        // No blast: first outputs track the first inputs.
        assert!((backend.pushed[0].left - input[0].left).abs() < 0.01);
    }

    #[test]
    fn controller_pins_nominal_without_capacity() {
        let mut controller = RateController::new();
        assert_eq!(controller.update(0, 0), 1.0);
        assert_eq!(controller.update(9999, 0), 1.0);
    }

    #[test]
    fn controller_saturates_at_both_rails() {
        let mut controller = RateController::new();
        assert_eq!(controller.update(0, 1000), 1.003);
        for _ in 0..100 {
            controller.update(0, 1000);
        }
        assert_eq!(controller.ratio(), 1.05);
        for _ in 0..100 {
            controller.update(1000, 1000);
        }
        assert_eq!(controller.ratio(), 0.95);
    }
}
