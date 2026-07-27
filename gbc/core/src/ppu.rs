use nerust_render_traits::FrameBuffer;

/// Stub PPU for Phase 3 compilation.
///
/// Struct definition and all method signatures are here; implementations
/// are no-ops or return dummy values. Filled in during Phase 6.
#[derive(Debug, Clone, Default)]
pub struct GbcPpu {
    _private: (), // prevent external construction until Phase 6
}

pub struct PpuStepResult {
    pub frame_done: bool,
    pub lcd_stat: bool,
    pub vblank: bool,
}

impl GbcPpu {
    pub fn step(&mut self, _cycles: u32) -> PpuStepResult {
        PpuStepResult {
            frame_done: false,
            lcd_stat: false,
            vblank: false,
        }
    }

    pub fn render(&self, _fb: &mut FrameBuffer) {}

    pub fn read_vram(&self, _addr: u16) -> u8 {
        0xFF
    }

    pub fn write_vram(&mut self, _addr: u16, _value: u8) {}

    pub fn read_oam(&self, _addr: u8) -> u8 {
        0xFF
    }

    pub fn write_oam(&mut self, _addr: u8, _value: u8) {}

    pub fn read_register(&self, _addr: u16) -> u8 {
        0xFF
    }

    pub fn write_register(&mut self, _addr: u16, _value: u8) {}

    pub fn read_palette(&self, _addr: u16) -> u8 {
        0xFF
    }

    pub fn write_palette(&mut self, _addr: u16, _value: u8) {}
}
