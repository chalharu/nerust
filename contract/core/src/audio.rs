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

/// 音量調整 + レンジ変換ラッパー
///
/// APU 出力 (0.0〜1.0) を AudioBackend 期待範囲 (-1.0〜1.0) に変換し、
/// ユーザー設定の音量を適用する。
/// フィルタ/リサンプラは持たない（ConsoleAudioBackend が担当）。
pub struct VolumeBackend {
    inner: Box<dyn AudioBackend>,
    volume: f32,
}

impl VolumeBackend {
    pub fn new(inner: Box<dyn AudioBackend>, volume: f32) -> Self {
        Self {
            inner,
            volume: volume.clamp(0.0, 1.0),
        }
    }
}

impl AudioBackend for VolumeBackend {
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
        self.inner.push((sample * 2.0 - 1.0) * self.volume);
    }
}
