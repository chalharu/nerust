use nerust_render_traits::FrameBuffer;

/// Stub PPU for Phase 3 compilation.
///
/// Minimal LY counter for VBlank polling support (needed by test ROMs).
/// Filled in during Phase 6.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct GbcPpu {
    ly: u8,
    hblank_counter: u32,
    frame_done: bool,
}

pub struct PpuStepResult {
    pub frame_done: bool,
    pub lcd_stat: bool,
    pub vblank: bool,
}

impl GbcPpu {
    pub fn step(&mut self, cycles: u32) -> PpuStepResult {
        let mut vblank = false;
        let lcd_stat = false;

        self.hblank_counter += cycles;
        // Each scanline is ~114 T-cycles (DMG). Advance LY on scanline boundary.
        while self.hblank_counter >= 114 {
            self.hblank_counter -= 114;
            self.ly = self.ly.wrapping_add(1);
            if self.ly >= 144 {
                vblank = true;
            }
        }

        let frame_done = self.frame_done;
        self.frame_done = false;

        PpuStepResult {
            frame_done,
            vblank,
            lcd_stat,
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

    pub fn read_register(&self, addr: u16) -> u8 {
        match addr {
            0xFF44 => self.ly,
            _ => 0xFF,
        }
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
    fn step_increments_ly() {
        let mut p = ppu();
        let r = p.step(114);
        assert_eq!(p.ly, 1);
        assert!(!r.vblank);
    }

    #[test]
    fn ly_reaches_vblank_region() {
        let mut p = ppu();
        p.step(114 * 144);
        let r = p.step(114);
        assert!(r.vblank);
    }

    #[test]
    fn render_is_noop() {
        let p = ppu();
        let mut fb = FrameBuffer::with_capacity(160, 144, nerust_render_traits::PixelFormat::Rgba);
        p.render(&mut fb);
    }

    #[test]
    fn read_ly_returns_value() {
        let mut p = ppu();
        p.step(114 * 10);
        assert_eq!(p.read_register(0xFF44), 10);
    }

    #[test]
    fn read_other_register_returns_0xff() {
        assert_eq!(ppu().read_register(0xFF40), 0xFF);
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
