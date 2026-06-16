use nerust_contract_core::audio::AudioBackend;
use nerust_soundfilter::Filter;
use nerust_soundfilter::NesFilter;
use nerust_soundfilter::resampler::Resampler;
use nerust_soundfilter::resampler::SimpleDownSampler;

const CLOCK_RATE: u32 = 1_789_773;
const OVERSAMPLE_FACTOR: u32 = 4;

fn oversampled_rate(device_rate: u32) -> u32 {
    device_rate
        .saturating_mul(OVERSAMPLE_FACTOR)
        .min(CLOCK_RATE)
        .max(device_rate)
}

/// コンソールレベルの音声ラッパー。
///
/// 内包する `AudioBackend` に対してオーバーサンプリング、
/// NES フィルタ、ダウンサンプリングを適用する。
/// `sample_rate()` はオーバーサンプリング後のレートを返すため、
/// コアは高いレートでサンプルを生成し、このラッパーがデバイスレートに
/// 変換する。レンジ変換 (0.0-1.0 → -1.0-1.0) やゲイン適用は行わない
/// （`VolumeBackend` など外側のラッパーの責務）。
pub struct ConsoleAudioBackend {
    inner: Box<dyn AudioBackend>,
    filter: NesFilter,
    resampler: SimpleDownSampler,
}

impl ConsoleAudioBackend {
    pub fn new(inner: Box<dyn AudioBackend>) -> Self {
        let device_rate = inner.sample_rate();
        let source_rate = oversampled_rate(device_rate);
        Self {
            inner,
            filter: NesFilter::new(device_rate as f32),
            resampler: SimpleDownSampler::new(
                f64::from(source_rate),
                f64::from(device_rate),
            ),
        }
    }
}

impl AudioBackend for ConsoleAudioBackend {
    fn start(&mut self) {
        self.inner.start();
    }

    fn pause(&mut self) {
        self.inner.pause();
    }

    fn sample_rate(&self) -> u32 {
        oversampled_rate(self.inner.sample_rate())
    }

    fn push(&mut self, sample: f32) {
        if let Some(resampled) = self.resampler.step(sample) {
            let filtered = self.filter.step(resampled);
            self.inner.push(filtered);
        }
    }
}
