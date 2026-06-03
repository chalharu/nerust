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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFilterProfile {
    Nes,
    Snes,
}
