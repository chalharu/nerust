use nerust_soundfilter::Filter;
use nerust_soundfilter::resampler::Resampler;

pub trait Sound {
    fn start(&mut self);
    fn pause(&mut self);
}

pub trait MixerInput {
    fn push(&mut self, data: f32); // 0.0 ~ 1.0

    /// Audio samples per second the mixer wants to receive from the core.
    ///
    /// The core advances APU timing at the CPU rate, but emits mixed audio only at this rate.
    /// Backends can request an oversampled rate here and downsample before device output when
    /// they need stronger anti-alias filtering.
    fn sample_rate(&self) -> u32 {
        48_000
    }
}

/// Multiplier cap on the internal oversampling rate relative to device rate.
const OVERSAMPLE_FACTOR: u32 = 4;

/// `AudioBackend` → `MixerInput` アダプタ
///
/// NES 固有のフィルタ (`NesFilter`) とリサンプラ (`SimpleDownSampler`) を内蔵し、
/// バックエンドに渡す前に NES APU 出力を処理する。
///
/// Phase 4b で `run_frame` が `&mut dyn AudioBackend` を直接受け取るようになった時点で
/// このアダプタは不要になり削除され、フィルタ/リサンプラは `NesConsole` 側に移動する。
pub struct MixerBridge {
    pub backend: Box<dyn nerust_contract_core::audio::AudioBackend + Send>,
    filter: nerust_soundfilter::NesFilter,
    gain: f32,
    resampler: nerust_soundfilter::resampler::SimpleDownSampler,
    source_sample_rate: u32,
}

impl MixerBridge {
    /// `output_rate` は NES CPU クロックレート。`sample_rate` は `backend.sample_rate()`
    /// から自動取得する。
    pub fn new(
        backend: Box<dyn nerust_contract_core::audio::AudioBackend + Send>,
        output_rate: u32,
        gain: f32,
    ) -> Self {
        let sample_rate = backend.sample_rate();
        let source_sample_rate = output_rate
            .min(sample_rate.saturating_mul(OVERSAMPLE_FACTOR))
            .max(sample_rate);
        Self {
            backend,
            filter: nerust_soundfilter::NesFilter::new(sample_rate as f32),
            gain,
            resampler: nerust_soundfilter::resampler::SimpleDownSampler::new(
                f64::from(source_sample_rate),
                f64::from(sample_rate),
            ),
            source_sample_rate,
        }
    }
}

impl MixerInput for MixerBridge {
    fn push(&mut self, data: f32) {
        if let Some(resampled) = self.resampler.step(data) {
            let sample = self.filter.step((resampled * 2.0 - 1.0) * self.gain);
            self.backend.push(sample);
        }
    }

    fn sample_rate(&self) -> u32 {
        self.source_sample_rate
    }
}

impl Sound for MixerBridge {
    fn start(&mut self) {
        self.backend.start();
    }

    fn pause(&mut self) {
        self.backend.pause();
    }
}
