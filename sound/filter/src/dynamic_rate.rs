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
//!
//! Control law: a wall-clock feed-forward base ratio (device samples per
//! wall second over nominal input samples per wall second, from cumulative
//! counters — burst quantization in the queue level vanishes as the totals
//! grow, so the base converges to the exact systematic offset and then
//! sits dead still) plus a gentle centering trim on the smoothed queue
//! fill. The trim is deliberately too weak to turn queue ripple into
//! pitch: a proportional-on-level controller with real authority hunts
//! audibly (±30 cents or more) under bursty device consumption.

use std::time::Instant;

use nerust_core_traits::audio::{AudioBackend, StereoSample};

/// Rate estimator holding the backend queue near half-full.
/// Output is the device-samples-per-core-sample ratio (1.0 = nominal).
#[derive(Debug, Clone)]
pub struct RateController {
    ratio: f32,
    target_fill: f32,
    max_adjust: f32,
    slew: f32,
    /// Nominal input samples produced since reset (feed-forward clock).
    produced: u64,
    /// Wall seconds accumulated alongside `produced` (frozen while the
    /// session produces nothing, so pause/hiccups cannot skew the base).
    elapsed: f64,
    /// Smoothed queue fill feeding the centering trim only.
    fill_smooth: f32,
    fill_alpha: f32,
    /// Trim authority per unit fill error — weak on purpose (see above).
    trim_gain: f32,
}

impl Default for RateController {
    fn default() -> Self {
        Self {
            ratio: 1.0,
            target_fill: 0.5,
            max_adjust: 0.05,
            slew: 0.003,
            produced: 0,
            elapsed: 0.0,
            fill_smooth: 0.5,
            fill_alpha: 0.1,
            trim_gain: 0.02,
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
        self.produced = 0;
        self.elapsed = 0.0;
        self.fill_smooth = self.target_fill;
    }

    /// Record one production batch: `input_len` nominal input samples
    /// took `dt_secs` wall seconds. Called once per pushed frame, before
    /// the control steps that consume it.
    pub fn note_production(&mut self, input_len: u64, dt_secs: f64) {
        self.produced = self.produced.saturating_add(input_len);
        if dt_secs > 0.0 && dt_secs.is_finite() {
            self.elapsed += dt_secs;
        }
    }

    /// Feed one control step's queue occupancy; returns the updated ratio.
    /// Unknown backends (`capacity == 0`) pin the ratio at nominal.
    /// `device_rate` is the backend's nominal drain rate (samples/second).
    pub fn update(&mut self, buffered: u64, capacity: u64, device_rate: u32) -> f32 {
        if capacity == 0 {
            self.ratio = 1.0;
            return self.ratio;
        }
        // Feed-forward base: exact systematic offset, ripple-free. Fades
        // in over 2..10s of production so startup stays bit-exact nominal.
        let fade = ((self.elapsed - 2.0) / 8.0).clamp(0.0, 1.0);
        let raw = if self.produced > 0 {
            f64::from(device_rate) * self.elapsed / self.produced as f64
        } else {
            1.0
        };
        let base = 1.0 + fade * (raw - 1.0);
        // Centering trim on the smoothed fill: walks the queue home over
        // seconds after transients, inaudible under ripple.
        let fill = (buffered as f32 / capacity as f32).clamp(0.0, 1.5);
        self.fill_smooth += (fill - self.fill_smooth) * self.fill_alpha;
        let trim = f64::from(self.trim_gain * (self.target_fill - self.fill_smooth));
        let target = (base + trim).clamp(
            1.0 - f64::from(self.max_adjust),
            1.0 + f64::from(self.max_adjust),
        );
        let delta =
            (target - f64::from(self.ratio)).clamp(-f64::from(self.slew), f64::from(self.slew));
        self.ratio = (f64::from(self.ratio) + delta) as f32;
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
    last_call: Option<Instant>,
}

impl DynamicRateFilter {
    pub fn new() -> Self {
        Self {
            controller: RateController::new(),
            prev: StereoSample::SILENCE,
            frac: 0.0,
            since_update: 0,
            last_call: None,
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
        self.last_call = None;
    }

    /// Stretch one frame's nominal samples against the backend queue
    /// level and push the result. The control step runs about once per
    /// frame worth of pushed samples (`sample_rate / 60`); between steps
    /// the last ratio holds. Wall time per call feeds the controller's
    /// feed-forward clock (see `RateController::note_production`).
    pub fn push_frame(&mut self, samples: &[StereoSample], backend: &mut dyn AudioBackend) {
        let now = Instant::now();
        let dt = self
            .last_call
            .replace(now)
            .map(|last| now.duration_since(last).as_secs_f64())
            .unwrap_or(0.0);
        self.controller.note_production(samples.len() as u64, dt);
        let interval = (backend.sample_rate() / 60).max(1) as u64;
        let device_rate = backend.sample_rate();
        let mut ratio = self.controller.ratio();
        for &sample in samples {
            self.since_update += 1;
            if self.since_update >= interval {
                self.since_update = 0;
                ratio = self.controller.update(
                    backend.buffered(),
                    backend.buffer_capacity(),
                    device_rate,
                );
            }
            if ratio == 1.0 {
                backend.push(sample);
            } else {
                self.frac += f64::from(ratio);
                while self.frac >= 1.0 {
                    self.frac -= 1.0;
                    // Clamp: when the carry crosses two boundaries at once
                    // the raw phase goes slightly negative (extrapolation
                    // just past `prev`); pin it to a clean repeat instead.
                    let t = (1.0 - self.frac).clamp(0.0, 1.0) as f32;
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
        assert_eq!(controller.update(0, 0, 48_000), 1.0);
        assert_eq!(controller.update(9999, 0, 44_100), 1.0);
    }

    #[test]
    fn startup_stays_bit_exact() {
        // Before 2s of production the feed-forward base is fully faded
        // out and the trim sees a half-full queue: ratio exactly 1.0,
        // so early frames pass through untouched.
        let mut controller = RateController::new();
        controller.note_production(800, 0.01);
        assert_eq!(controller.update(2400, 4800, 48_000), 1.0);
    }

    #[test]
    fn base_converges_to_systematic_offset_and_holds_steady() {
        // Regression test for audible pitch wobble: a GBA-paced session
        // produces 804 nominal samples per 1/60s while the device drains
        // 799, with zero-mean ±150 burst ripple on the drain. A
        // proportional-on-level law turns that ripple into ±30-cent pitch
        // swings; the feed-forward base must sit still once converged.
        let mut controller = RateController::new();
        let mut queued = 2400.0f64;
        let mut ratios = Vec::new();
        for step in 0..7200 {
            controller.note_production(804, 1.0 / 60.0);
            let ratio = controller.update(queued as u64, 4800, 48_000);
            ratios.push(ratio);
            let drain = 799.0 + if step % 12 < 6 { 150.0 } else { -150.0 };
            queued = (queued + 804.0 * f64::from(ratio) - drain).clamp(0.0, 4800.0);
        }
        let tail = &ratios[6000..];
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(
            (mean - 799.0 / 804.0).abs() < 0.0005,
            "base must learn the systematic offset, mean={mean}"
        );
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for &ratio in tail {
            lo = lo.min(ratio);
            hi = hi.max(ratio);
        }
        assert!(
            hi - lo < 0.0015,
            "steady-state wobble must stay under ~±1 cent: [{lo}, {hi}]"
        );
    }

    #[test]
    fn trim_recenters_full_queue_without_drama() {
        // Starting from a pinned-full queue, the trim walks the level
        // home over minutes without ever railing the ratio.
        let mut controller = RateController::new();
        let mut queued = 4800.0f64;
        for step in 0..3600 {
            controller.note_production(804, 1.0 / 60.0);
            let ratio = controller.update(queued as u64, 4800, 48_000);
            assert!((0.96..=1.0).contains(&ratio), "step {step}: ratio={ratio}");
            let drain = 799.0 + if step % 12 < 6 { 150.0 } else { -150.0 };
            queued = (queued + 804.0 * f64::from(ratio) - drain).clamp(0.0, 4800.0);
        }
        let fill = queued / 4800.0;
        assert!((0.3..0.7).contains(&fill), "fill={fill}");
    }

    #[test]
    fn base_clamps_at_rails() {
        let mut controller = RateController::new();
        // Emulation far behind wall clock: raw base explodes, clamp and
        // slew bound it at +5%.
        for _ in 0..60 {
            controller.note_production(80, 1.0);
            controller.update(2400, 4800, 48_000);
        }
        assert_eq!(controller.ratio(), 1.05);
        // ...and far ahead floors it at -5%.
        for _ in 0..200 {
            controller.note_production(480_000, 0.001);
            controller.update(2400, 4800, 48_000);
        }
        assert_eq!(controller.ratio(), 0.95);
    }
}
