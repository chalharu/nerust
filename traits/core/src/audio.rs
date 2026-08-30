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

    /// 再生音量を 0.0〜1.0 の範囲で設定する。
    ///
    /// デフォルト実装は no-op。`GainBackend` が `set_gain()` に委譲する。
    fn set_volume(&mut self, _volume: f32) {}
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

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn push(&mut self, sample: StereoSample) {
        self.inner.push(sample.scale(self.gain));
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
}
