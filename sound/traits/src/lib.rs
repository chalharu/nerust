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

/// `AudioBackend` → `MixerInput` アダプタ
///
/// Phase 4b で `run_frame` が `&mut dyn AudioBackend` を直接受け取るようになった時点で
/// このアダプタは不要になり削除される。それまでは既存の `MixerInput` を要求する
/// `run_frame` に `AudioBackend` を渡すための橋渡しとして使う。
pub struct MixerBridge {
    pub backend: Box<dyn nerust_contract_core::audio::AudioBackend + Send>,
}

impl MixerInput for MixerBridge {
    fn push(&mut self, data: f32) {
        self.backend.push(data);
    }

    fn sample_rate(&self) -> u32 {
        self.backend.sample_rate()
    }
}
