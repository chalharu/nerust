/// GBA APU - Phase 9 stub
/// Sound registers are currently handled in `GbaMemoryBus` (soundcnt, wave_ram).
/// This crate will host PSG, FIFO and mixer in Phase 9.
#[derive(Debug, Default)]
pub struct GbaApu {
    _placeholder: u8,
}

impl GbaApu {
    pub fn new() -> Self { Self::default() }
}
