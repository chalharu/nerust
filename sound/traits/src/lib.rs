use nerust_contract_core::audio::AudioBackend;
use nerust_soundfilter::Filter;
use nerust_soundfilter::resampler::Resampler;

/// Multiplier cap on the internal oversampling rate relative to device rate.
const OVERSAMPLE_FACTOR: u32 = 4;

/// NES APU 出力をダウンサンプリング・フィルタリングして `AudioBackend` に渡すアダプタ。
///
/// `AudioBackend` をラップし、CPU クロックレートで到着するサンプルを
/// バックエンドの出力レートにリサンプルする。`sample_rate()` は CPU クロックレートを
/// 返すことで、APU が全サンプルを送出することを保証する。
///
/// 将来 Phase 2c-3 以降でフィルタ/リサンプラは `NesConsole` 側に移動し、
/// このアダプタは削除される。
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

impl AudioBackend for MixerBridge {
    fn start(&mut self) {
        self.backend.start();
    }

    fn pause(&mut self) {
        self.backend.pause();
    }

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


