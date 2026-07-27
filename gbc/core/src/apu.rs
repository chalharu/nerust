/// Stub APU for Phase 3 compilation.
///
/// All methods are no-ops. Filled in during Phase 7.
#[derive(Debug, Clone, Default)]
pub struct GbcApu {
    _private: (),
}

impl GbcApu {
    pub fn step(&mut self, _cycles: u32) {}

    pub fn flush_samples(&mut self) -> Vec<f32> {
        Vec::new()
    }

    pub fn read_register(&self, _addr: u16) -> u8 {
        0xFF
    }

    pub fn write_register(&mut self, _addr: u16, _value: u8) {}
}
