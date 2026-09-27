use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StereoSample {
    pub left: f32,
    pub right: f32,
}

impl StereoSample {
    pub const SILENCE: Self = Self::new(0.0, 0.0);

    pub const fn new(left: f32, right: f32) -> Self {
        Self { left, right }
    }

    pub const fn mono(sample: f32) -> Self {
        Self::new(sample, sample)
    }

    pub fn downmix(self) -> f32 {
        (self.left + self.right) * 0.5
    }

    pub fn scale(self, gain: f32) -> Self {
        Self::new(self.left * gain, self.right * gain)
    }
}

pub trait AudioBackend: Send {
    fn start(&mut self);
    fn pause(&mut self);
    fn sample_rate(&self) -> u32 {
        48_000
    }
    fn push(&mut self, sample: StereoSample);

    /// Drop and re-acquire the OS audio stream with identical parameters.
    /// Backends whose streams can die underneath them (AAudio
    /// `Disconnected` after device/route change or server reclaim across
    /// app suspend) must recreate the stream here: a dead stream can
    /// never be revived with `start()`. Implementations must keep
    /// reporting the same sample rate (cores validate save states
    /// against it) and should drop stale queued samples. Default is a
    /// no-op for backends that never lose their stream.
    fn reconnect(&mut self) {}

    /// Samples currently queued for the device (device callback has not
    /// consumed them yet). Drives dynamic rate control: cores stretch the
    /// resample ratio to hold the queue near half of `buffer_capacity`.
    /// Default 0 (unknown) disables rate control (see `RateController`).
    fn buffered(&self) -> u64 {
        0
    }

    /// Queue capacity in samples matching `buffered`. Default 0 (unknown).
    fn buffer_capacity(&self) -> u64 {
        0
    }

    /// 再生音量を 0.0〜1.0 の範囲で設定する。
    ///
    /// デフォルト実装は no-op。`GainBackend` が `set_gain()` に委譲する。
    fn set_volume(&mut self, _volume: f32) {}
}

/// Dynamic rate control (RetroArch-style): holds the backend queue near
/// half-full by stretching the core's resample ratio a few percent.
/// When emulation runs slow the queue drains and the ratio rises above
/// 1 (more device samples per emulated second: pitch rises slightly
/// instead of gaping); when emulation runs ahead it falls below 1.
/// Slew-limited so the pitch glides instead of jumping; clamped to
/// ±5% (≈0.8 semitones worst case, transient only).
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

    /// Current output/input sample ratio (1.0 = nominal rate).
    pub fn ratio(&self) -> f32 {
        self.ratio
    }

    /// Back to nominal (call on load/unload: the queue no longer matches
    /// the pre-switch emulation timeline).
    pub fn reset(&mut self) {
        self.ratio = 1.0;
    }

    /// Feed one frame's queue occupancy; returns the updated ratio.
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

/// Factory for creating and probing audio backends.
///
/// Implementations should be zero-sized types (ZST) stored as `&'static`
/// references so that registration requires no heap allocation.
pub trait AudioBackendFactory: Send + Sync {
    fn name(&self) -> &'static str;
    /// Returns the sample rates this backend supports on the current hardware.
    fn probe(&self) -> Vec<u32>;
    /// Attempts to create a backend. Returns `None` on failure.
    fn build(&self, sample_rate: u32, latency_ms: u32) -> Option<Box<dyn AudioBackend>>;
}

/// Registry of audio backend factories.
///
/// Backends are registered with a priority (lower = tried first).
/// `autoselect` tries each factory in priority order and returns
/// the first successfully created backend, falling back to `NullAudio`.
/// `supported_rates` lazily probes all factories on first access and
/// caches the result.
#[derive(Default)]
pub struct AudioBackendRegistry {
    entries: Vec<BackendEntry>,
    probed: OnceLock<Vec<u32>>,
}

struct BackendEntry {
    priority: u8,
    factory: Box<dyn AudioBackendFactory>,
}

impl AudioBackendRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, priority: u8, factory: Box<dyn AudioBackendFactory>) {
        self.entries.push(BackendEntry { priority, factory });
    }

    /// Returns the supported sample rates by probing registered factories.
    ///
    /// Factories are tried in priority order; the first non-empty result is
    /// cached and returned for all subsequent calls.
    ///
    /// The returned slice is **always sorted in ascending order** so that
    /// callers can safely use `.last()` to obtain the highest rate or
    /// `.first()` for the lowest.
    pub fn supported_rates(&self) -> &[u32] {
        self.probed.get_or_init(|| {
            let mut sorted: Vec<&BackendEntry> = self.entries.iter().collect();
            sorted.sort_by_key(|e| e.priority);
            for entry in sorted {
                let mut rates = entry.factory.probe();
                if !rates.is_empty() {
                    rates.sort();
                    return rates;
                }
            }
            Vec::new()
        })
    }

    pub fn autoselect(&self, sample_rate: u32, latency_ms: u32) -> Box<dyn AudioBackend> {
        let mut entries: Vec<&BackendEntry> = self.entries.iter().collect();
        entries.sort_by_key(|e| e.priority);
        for entry in entries {
            if let Some(backend) = entry.factory.build(sample_rate, latency_ms) {
                log::info!("autoselect: selected {}", entry.factory.name());
                return backend;
            }
        }
        Box::new(NullAudio)
    }
}

/// 無音出力バックエンド
///
/// 常に利用可能で、テスト・CI・ヘッドレス動作に使用する。
pub struct NullAudio;

impl AudioBackend for NullAudio {
    fn start(&mut self) {}
    fn pause(&mut self) {}
    fn push(&mut self, _sample: StereoSample) {}
}

/// ゲイン適用ラッパー。`AudioBackend` に gain を乗算してから渡す。
///
/// Sample rate / start / pause は inner に委譲する。
pub struct GainBackend {
    inner: Box<dyn AudioBackend>,
    gain: f32,
}

impl GainBackend {
    pub fn new(inner: Box<dyn AudioBackend>, gain: f32) -> Self {
        Self { inner, gain }
    }
}

impl AudioBackend for GainBackend {
    fn start(&mut self) {
        self.inner.start();
    }

    fn pause(&mut self) {
        self.inner.pause();
    }

    fn reconnect(&mut self) {
        self.inner.reconnect();
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn push(&mut self, sample: StereoSample) {
        self.inner.push(sample.scale(self.gain));
    }

    fn buffered(&self) -> u64 {
        self.inner.buffered()
    }

    fn buffer_capacity(&self) -> u64 {
        self.inner.buffer_capacity()
    }

    fn set_volume(&mut self, volume: f32) {
        self.gain = volume;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    struct Capture(Arc<Mutex<Vec<StereoSample>>>);

    impl AudioBackend for Capture {
        fn start(&mut self) {}
        fn pause(&mut self) {}
        fn push(&mut self, sample: StereoSample) {
            self.0.lock().unwrap().push(sample);
        }
    }

    #[test]
    fn stereo_sample_helpers_preserve_channels() {
        assert_eq!(StereoSample::mono(0.25), StereoSample::new(0.25, 0.25));
        assert_eq!(StereoSample::new(0.25, 0.75).downmix(), 0.5);
        assert_eq!(
            StereoSample::new(0.25, -0.5).scale(0.5),
            StereoSample::new(0.125, -0.25)
        );
    }

    #[test]
    fn gain_backend_scales_channels_independently() {
        let samples = Arc::new(Mutex::new(Vec::new()));
        let mut backend = GainBackend::new(Box::new(Capture(Arc::clone(&samples))), 0.5);
        backend.push(StereoSample::new(0.8, -0.4));
        assert_eq!(
            samples.lock().unwrap().as_slice(),
            &[StereoSample::new(0.4, -0.2)]
        );
    }

    #[test]
    fn gain_backend_delegates_reconnect() {
        use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

        struct ReconnectProbe {
            reconnected: Arc<AtomicBool>,
        }
        impl AudioBackend for ReconnectProbe {
            fn start(&mut self) {}
            fn pause(&mut self) {}
            fn push(&mut self, _sample: StereoSample) {}
            fn reconnect(&mut self) {
                self.reconnected.store(true, SeqCst);
            }
        }
        let reconnected = Arc::new(AtomicBool::new(false));
        let mut backend = GainBackend::new(
            Box::new(ReconnectProbe {
                reconnected: Arc::clone(&reconnected),
            }),
            0.5,
        );
        backend.reconnect();
        assert!(reconnected.load(SeqCst));
    }

    #[test]
    fn rate_controller_holds_nominal_without_capacity() {
        let mut controller = RateController::new();
        assert_eq!(controller.ratio(), 1.0);
        // Unknown backend: pinned at nominal, no stale stretch.
        assert_eq!(controller.update(0, 0), 1.0);
        assert_eq!(controller.update(9999, 0), 1.0);
    }

    #[test]
    fn rate_controller_speeds_up_when_starved() {
        let mut controller = RateController::new();
        // Empty queue: target saturates at +5%, approached at the slew rate.
        assert_eq!(controller.update(0, 1000), 1.003);
        for _ in 0..100 {
            controller.update(0, 1000);
        }
        assert_eq!(controller.ratio(), 1.05);
    }

    #[test]
    fn rate_controller_slows_down_when_flooded() {
        let mut controller = RateController::new();
        // Full queue: target saturates at -5%.
        assert_eq!(controller.update(1000, 1000), 0.997);
        for _ in 0..100 {
            controller.update(1000, 1000);
        }
        assert_eq!(controller.ratio(), 0.95);
    }

    #[test]
    fn rate_controller_rests_near_nominal_at_half_fill() {
        let mut controller = RateController::new();
        for _ in 0..100 {
            controller.update(500, 1000);
        }
        assert!((controller.ratio() - 1.0).abs() < 0.001);
    }

    #[test]
    fn rate_controller_reset_returns_to_nominal() {
        let mut controller = RateController::new();
        for _ in 0..100 {
            controller.update(0, 1000);
        }
        assert!(controller.ratio() > 1.0);
        controller.reset();
        assert_eq!(controller.ratio(), 1.0);
    }
}
