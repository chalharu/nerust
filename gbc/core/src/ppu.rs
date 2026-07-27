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

#[cfg(test)]
mod tests {
    use super::*;

    fn ppu() -> GbcPpu {
        GbcPpu::default()
    }

    #[test]
    fn step_returns_no_result_defaults() {
        let mut p = ppu();
        let r = p.step(100);
        assert!(!r.frame_done);
        assert!(!r.lcd_stat);
        assert!(!r.vblank);
    }

    #[test]
    fn render_is_noop() {
        let p = ppu();
        let mut fb = FrameBuffer::with_capacity(160, 144, nerust_render_traits::PixelFormat::Rgba);
        p.render(&mut fb);
    }

    #[test]
    fn read_vram_returns_0xff() {
        assert_eq!(ppu().read_vram(0x8000), 0xFF);
    }

    #[test]
    fn write_vram_is_noop() {
        ppu().write_vram(0x8000, 0x42);
    }

    #[test]
    fn read_oam_returns_0xff() {
        assert_eq!(ppu().read_oam(0), 0xFF);
    }

    #[test]
    fn write_oam_is_noop() {
        ppu().write_oam(0, 0x42);
    }

    #[test]
    fn read_register_returns_0xff() {
        assert_eq!(ppu().read_register(0xFF40), 0xFF);
    }

    #[test]
    fn write_register_is_noop() {
        ppu().write_register(0xFF40, 0x91);
    }

    #[test]
    fn read_palette_returns_0xff() {
        assert_eq!(ppu().read_palette(0xFF68), 0xFF);
    }

    #[test]
    fn write_palette_is_noop() {
        ppu().write_palette(0xFF68, 0x7F);
    }
}
