pub trait AudioBackend: Send {
    fn start(&mut self);
    fn pause(&mut self);
    fn sample_rate(&self) -> u32 {
        48_000
    }
    fn push(&mut self, sample: f32);
}

/// Registry of audio backend factories.
///
/// Backends are registered with a priority (lower = tried first).
/// `autoselect` tries each factory in priority order and returns
/// the first successfully created backend, falling back to `NullAudio`.
pub struct AudioBackendRegistry {
    entries: Vec<BackendEntry>,
}

struct BackendEntry {
    priority: u8,
    name: &'static str,
    factory: fn(u32, u32) -> Option<Box<dyn AudioBackend>>,
}

impl Default for AudioBackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackendRegistry {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn register(
        &mut self,
        priority: u8,
        name: &'static str,
        factory: fn(u32, u32) -> Option<Box<dyn AudioBackend>>,
    ) {
        self.entries.push(BackendEntry {
            priority,
            name,
            factory,
        });
    }

    pub fn autoselect(&self, sample_rate: u32, latency_ms: u32) -> Box<dyn AudioBackend> {
        let mut entries: Vec<&BackendEntry> = self.entries.iter().collect();
        entries.sort_by_key(|e| e.priority);
        for entry in entries {
            if let Some(backend) = (entry.factory)(sample_rate, latency_ms) {
                log::info!("autoselect: selected {}", entry.name);
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
    fn push(&mut self, _sample: f32) {}
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

    /// gain を動的に変更する (session rebuild 不要)。
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
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

    fn push(&mut self, sample: f32) {
        self.inner.push(sample * self.gain);
    }
}
