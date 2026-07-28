use nerust_render_traits::FrameBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PpuMode {
    HBlank = 0,
    VBlank = 1,
    OamSearch = 2,
    PixelTransfer = 3,
}

const T_CYCLES_PER_SCANLINE: u32 = 114;
const T_CYCLES_OAM_SEARCH: u32 = 20;
const T_CYCLES_PIXEL_TRANSFER: u32 = 43;
const SCANLINES_PER_FRAME: u8 = 154;
const VBLANK_START: u8 = 144;

pub struct PpuStepResult {
    pub frame_done: bool,
    pub lcd_stat: bool,
    pub vblank: bool,
}

pub struct GbcPpu {
    lcdc: u8,
    stat: u8,
    scy: u8,
    scx: u8,
    ly: u8,
    lyc: u8,
    wy: u8,
    wx: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,

    vbk: u8,
    bgpi: u8,
    bgpd: u8,
    obpi: u8,
    obpd: u8,
    opri: u8,

    vram: [u8; 0x4000],
    oam: [u8; 160],
    bg_palette: [u16; 32],
    obj_palette: [u16; 32],

    mode_clock: u32,
    frame_complete: bool,
}

impl Default for GbcPpu {
    fn default() -> Self {
        Self {
            lcdc: 0x91,
            stat: 0x00,
            scy: 0x00,
            scx: 0x00,
            ly: 0x00,
            lyc: 0x00,
            wy: 0x00,
            wx: 0x00,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            vbk: 0,
            bgpi: 0,
            bgpd: 0,
            obpi: 0,
            obpd: 0,
            opri: 0,
            vram: [0; 0x4000],
            oam: [0; 160],
            bg_palette: [0; 32],
            obj_palette: [0; 32],
            mode_clock: 0,
            frame_complete: false,
        }
    }
}

impl GbcPpu {
    pub fn step(&mut self, cycles: u32) -> PpuStepResult {
        let frame_done = self.frame_complete;
        self.frame_complete = false;

        if self.lcdc & 0x80 == 0 {
            self.ly = 0;
            self.mode_clock = 0;
            return PpuStepResult {
                frame_done,
                lcd_stat: false,
                vblank: false,
            };
        }

        let mut lcd_stat = false;
        let mut vblank = false;

        self.mode_clock += cycles;

        let current_mode = if self.ly >= VBLANK_START {
            PpuMode::VBlank
        } else {
            let t = self.mode_clock;
            if t <= T_CYCLES_OAM_SEARCH {
                PpuMode::OamSearch
            } else if t <= T_CYCLES_OAM_SEARCH + T_CYCLES_PIXEL_TRANSFER {
                PpuMode::PixelTransfer
            } else {
                PpuMode::HBlank
            }
        };

        while self.mode_clock >= T_CYCLES_PER_SCANLINE {
            self.mode_clock -= T_CYCLES_PER_SCANLINE;
            self.ly = self.ly.wrapping_add(1);

            if self.ly >= VBLANK_START {
                vblank = true;
            }

            if self.ly >= SCANLINES_PER_FRAME {
                self.ly = 0;
                self.frame_complete = true;
            }

            self.check_lyc(&mut lcd_stat);
        }

        let mode_val = match current_mode {
            PpuMode::HBlank => 0,
            PpuMode::VBlank => 1,
            PpuMode::OamSearch => 2,
            PpuMode::PixelTransfer => 3,
        };
        self.stat = (self.stat & 0xFC) | mode_val;
        self.check_lyc(&mut lcd_stat);

        if current_mode == PpuMode::VBlank && (self.stat & 0x10) != 0 {
            lcd_stat = true;
        }
        if current_mode == PpuMode::OamSearch && (self.stat & 0x20) != 0 {
            lcd_stat = true;
        }
        if current_mode == PpuMode::HBlank && (self.stat & 0x08) != 0 {
            lcd_stat = true;
        }

        PpuStepResult {
            frame_done: self.frame_complete,
            lcd_stat,
            vblank,
        }
    }

    fn check_lyc(&mut self, lcd_stat: &mut bool) {
        let coincide = self.ly == self.lyc;
        let bit2 = if coincide { 0x04 } else { 0x00 };
        self.stat = (self.stat & !0x04) | bit2;
        if coincide && (self.stat & 0x40) != 0 {
            *lcd_stat = true;
        }
    }

    pub fn render(&self, _fb: &mut FrameBuffer) {}

    pub fn read_vram(&self, addr: u16) -> u8 {
        let idx = if self.vbk == 0 {
            addr & 0x1FFF
        } else {
            0x2000 + (addr & 0x1FFF)
        };
        self.vram[idx as usize]
    }

    pub fn write_vram(&mut self, addr: u16, value: u8) {
        let idx = if self.vbk == 0 {
            addr & 0x1FFF
        } else {
            0x2000 + (addr & 0x1FFF)
        };
        self.vram[idx as usize] = value;
    }

    pub fn read_oam(&self, addr: u8) -> u8 {
        self.oam[addr as usize]
    }

    pub fn write_oam(&mut self, addr: u8, value: u8) {
        self.oam[addr as usize] = value;
    }

    pub fn read_register(&self, addr: u16) -> u8 {
        match addr {
            0xFF40 => self.lcdc,
            0xFF41 => (self.stat & 0x78) | 0x80 | (self.stat & 0x07),
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.ly,
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,
            0xFF4F => 0xFE | self.vbk,
            0xFF68 => self.bgpi | 0x40,
            0xFF69 => {
                let idx = (self.bgpi & 0x3F) as usize;
                let pal = self.bg_palette[idx >> 1];
                if idx & 1 == 0 {
                    pal as u8
                } else {
                    (pal >> 8) as u8
                }
            }
            0xFF6A => self.obpi | 0x40,
            0xFF6B => {
                let idx = (self.obpi & 0x3F) as usize;
                let pal = self.obj_palette[idx >> 1];
                if idx & 1 == 0 {
                    pal as u8
                } else {
                    (pal >> 8) as u8
                }
            }
            0xFF6C => self.opri | 0xFE,
            _ => 0xFF,
        }
    }

    pub fn write_register(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF40 => self.lcdc = value,
            0xFF41 => self.stat = (self.stat & 0x07) | (value & 0x78),
            0xFF42 => self.scy = value,
            0xFF43 => self.scx = value,
            0xFF45 => self.lyc = value,
            0xFF47 => self.bgp = value,
            0xFF48 => self.obp0 = value,
            0xFF49 => self.obp1 = value,
            0xFF4A => self.wy = value,
            0xFF4B => self.wx = value,
            0xFF4F => self.vbk = value & 0x01,
            0xFF68 => {
                self.bgpi = value & 0x3F;
                if value & 0x80 != 0 {
                    self.bgpi |= 0x80;
                }
            }
            0xFF69 => {
                let idx = (self.bgpi & 0x3F) as usize;
                let auto_inc = self.bgpi & 0x80 != 0;
                if idx & 1 == 0 {
                    self.bg_palette[idx >> 1] = (self.bg_palette[idx >> 1] & 0xFF00) | value as u16;
                } else {
                    self.bg_palette[idx >> 1] =
                        (self.bg_palette[idx >> 1] & 0x00FF) | (value as u16) << 8;
                }
                if auto_inc {
                    self.bgpi = (self.bgpi & 0x80) | ((self.bgpi + 1) & 0x3F);
                }
            }
            0xFF6A => {
                self.obpi = value & 0x3F;
                if value & 0x80 != 0 {
                    self.obpi |= 0x80;
                }
            }
            0xFF6B => {
                let idx = (self.obpi & 0x3F) as usize;
                let auto_inc = self.obpi & 0x80 != 0;
                if idx & 1 == 0 {
                    self.obj_palette[idx >> 1] =
                        (self.obj_palette[idx >> 1] & 0xFF00) | value as u16;
                } else {
                    self.obj_palette[idx >> 1] =
                        (self.obj_palette[idx >> 1] & 0x00FF) | (value as u16) << 8;
                }
                if auto_inc {
                    self.obpi = (self.obpi & 0x80) | ((self.obpi + 1) & 0x3F);
                }
            }
            0xFF6C => self.opri = value & 0x01,
            _ => {}
        }
    }

    pub fn read_palette(&self, addr: u16) -> u8 {
        match addr {
            0xFF68 | 0xFF69 => self.read_register(addr),
            0xFF6A | 0xFF6B => self.read_register(addr),
            _ => 0xFF,
        }
    }

    pub fn write_palette(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF68 | 0xFF69 => self.write_register(addr, value),
            0xFF6A | 0xFF6B => self.write_register(addr, value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppu() -> GbcPpu {
        GbcPpu::default()
    }

    fn step_ly(p: &mut GbcPpu, target_ly: u8) {
        while p.ly != target_ly {
            p.step(T_CYCLES_PER_SCANLINE);
        }
    }

    #[test]
    fn step_increments_ly() {
        let mut p = ppu();
        let r = p.step(T_CYCLES_PER_SCANLINE);
        assert_eq!(p.ly, 1);
        assert!(!r.vblank);
    }

    #[test]
    fn ly_reaches_vblank_region() {
        let mut p = ppu();
        step_ly(&mut p, VBLANK_START);
        let r = p.step(T_CYCLES_PER_SCANLINE);
        assert!(r.vblank);
    }

    #[test]
    fn ly_wraps_at_153() {
        let mut p = ppu();
        step_ly(&mut p, 153);
        let r = p.step(T_CYCLES_PER_SCANLINE);
        assert_eq!(p.ly, 0);
        assert!(r.frame_done);
    }

    #[test]
    fn read_ly_returns_value() {
        let mut p = ppu();
        step_ly(&mut p, 10);
        assert_eq!(p.read_register(0xFF44), 10);
    }

    #[test]
    fn lyc_coincidence_sets_stat_bit2() {
        let mut p = ppu();
        p.write_register(0xFF45, 3);
        step_ly(&mut p, 3);
        assert!(p.read_register(0xFF41) & 0x04 != 0);
    }

    #[test]
    fn lyc_coincidence_clears_when_ly_passes() {
        let mut p = ppu();
        p.write_register(0xFF45, 3);
        step_ly(&mut p, 3);
        assert!(p.read_register(0xFF41) & 0x04 != 0);
        step_ly(&mut p, 4);
        assert!(p.read_register(0xFF41) & 0x04 == 0);
    }

    #[test]
    fn lcd_off_resets_ly() {
        let mut p = ppu();
        step_ly(&mut p, 100);
        p.write_register(0xFF40, 0x00);
        p.step(T_CYCLES_PER_SCANLINE);
        assert_eq!(p.ly, 0);
    }

    #[test]
    fn stat_write_preserves_lower_bits() {
        let mut p = ppu();
        p.write_register(0xFF41, 0xFF);
        assert_eq!(p.read_register(0xFF41) & 0x07, 0);
        assert_eq!(p.read_register(0xFF41) & 0x78, 0x78);
    }

    #[test]
    fn vblank_interrupt_fires_during_vblank() {
        let mut p = ppu();
        p.write_register(0xFF41, 0x10);
        step_ly(&mut p, VBLANK_START);
        let r = p.step(T_CYCLES_PER_SCANLINE);
        assert!(r.vblank);
        assert!(r.lcd_stat);
    }

    #[test]
    fn lyc_interrupt_fires_on_coincidence() {
        let mut p = ppu();
        p.write_register(0xFF41, 0x40);
        p.write_register(0xFF45, 5);
        step_ly(&mut p, 4);
        let r = p.step(T_CYCLES_PER_SCANLINE);
        assert!(r.lcd_stat);
    }
}
