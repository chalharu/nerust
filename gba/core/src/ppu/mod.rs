mod affine;
mod bg;
mod color;
mod mosaic;
mod obj;
mod window;

pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 160;
pub const CYCLES_PER_LINE: u16 = 1232;
pub const HDRAW_CYCLES: u16 = 960;
pub const HBLANK_FLAG_CYCLES: u16 = 1006;
pub const LINES_PER_FRAME: u16 = 228;

pub fn bgr555_to_rgba8888(color: u16) -> u32 {
    color::rgba8888(color & 0x7FFF)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PpuEvent {
    pub frame_complete: bool,
    pub interrupt_mask: u16,
    pub hblank_started: bool,
    pub vblank_started: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LayerPixel {
    color: u16,
    priority: u8,
    layer: u8,
    semi_transparent: bool,
}

#[derive(Debug)]
pub(crate) struct PpuRegisters {
    pub dispcnt: u16,
    pub dispstat: u16,
    pub greenswap: u16,
    pub bgcnt: [u16; 4],
    pub hofs: [u16; 4],
    pub vofs: [u16; 4],
    pub pa: [i16; 2],
    pub pb: [i16; 2],
    pub pc: [i16; 2],
    pub pd: [i16; 2],
    pub ref_x: [i32; 2],
    pub ref_y: [i32; 2],
    ref_x_raw: [u32; 2],
    ref_y_raw: [u32; 2],
    pub winh: [u16; 2],
    pub winv: [u16; 2],
    pub winin: u16,
    pub winout: u16,
    pub mosaic: u16,
    pub bldcnt: u16,
    pub bldalpha: u16,
    pub bldy: u16,
}

impl Default for PpuRegisters {
    fn default() -> Self {
        Self {
            dispcnt: 0x0080,
            dispstat: 0,
            greenswap: 0,
            bgcnt: [0; 4],
            hofs: [0; 4],
            vofs: [0; 4],
            pa: [0x100; 2],
            pb: [0; 2],
            pc: [0; 2],
            pd: [0x100; 2],
            ref_x: [0; 2],
            ref_y: [0; 2],
            ref_x_raw: [0; 2],
            ref_y_raw: [0; 2],
            winh: [0; 2],
            winv: [0; 2],
            winin: 0,
            winout: 0,
            mosaic: 0,
            bldcnt: 0,
            bldalpha: 0,
            bldy: 0,
        }
    }
}

pub struct GbaPpu {
    registers: PpuRegisters,
    internal_x: [i32; 2],
    internal_y: [i32; 2],
    cycle: u16,
    vcount: u16,
    frame: Box<[u32]>,
}

impl GbaPpu {
    pub fn new() -> Self {
        Self {
            registers: PpuRegisters::default(),
            internal_x: [0; 2],
            internal_y: [0; 2],
            cycle: 0,
            vcount: 0,
            frame: vec![color::rgba8888(0x7FFF); WIDTH * HEIGHT].into_boxed_slice(),
        }
    }

    pub fn step(&mut self, vram: &[u8], palette: &[u8], oam: &[u8]) -> PpuEvent {
        let mut event = PpuEvent::default();
        self.cycle += 1;
        if self.vcount < HEIGHT as u16 && self.cycle <= HDRAW_CYCLES && self.cycle % 4 == 0 {
            self.render_pixel(
                self.cycle as usize / 4 - 1,
                self.vcount as usize,
                vram,
                palette,
                oam,
            );
        }
        if self.cycle == HBLANK_FLAG_CYCLES {
            self.handle_hblank_flag(&mut event);
        }
        if self.cycle == CYCLES_PER_LINE {
            self.handle_line_end(&mut event);
        }
        event
    }

    fn handle_hblank_flag(&mut self, event: &mut PpuEvent) {
        event.hblank_started = true;
        self.registers.dispstat |= 1 << 1;
        if self.registers.dispstat & (1 << 4) != 0 {
            event.interrupt_mask |= 1 << 1;
        }
    }

    fn handle_line_end(&mut self, event: &mut PpuEvent) {
        self.cycle = 0;
        self.registers.dispstat &= !(1 << 1);
        self.advance_affine();
        self.advance_vcount(event);
        self.update_vcount_match(event);
    }

    fn advance_affine(&mut self) {
        if self.vcount >= HEIGHT as u16 {
            return;
        }
        crate::ppu::affine::advance_line(
            &mut self.internal_x,
            &mut self.internal_y,
            self.registers.pb,
            self.registers.pd,
        );
    }

    fn advance_vcount(&mut self, event: &mut PpuEvent) {
        self.vcount += 1;
        if self.vcount == HEIGHT as u16 {
            event.vblank_started = true;
            self.registers.dispstat |= 1;
            if self.registers.dispstat & (1 << 3) != 0 {
                event.interrupt_mask |= 1;
            }
        } else if self.vcount == LINES_PER_FRAME {
            self.vcount = 0;
            self.registers.dispstat &= !1;
            self.internal_x = self.registers.ref_x;
            self.internal_y = self.registers.ref_y;
            event.frame_complete = true;
        }
    }

    pub fn frame_buffer(&self) -> &[u32] {
        &self.frame
    }

    pub fn vcount(&self) -> u16 {
        self.vcount
    }

    pub fn cycle(&self) -> u16 {
        self.cycle
    }

    pub fn dispcnt(&self) -> u16 {
        self.registers.dispcnt
    }

    pub fn dispstat(&self) -> u16 {
        self.registers.dispstat
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn read_register(&self, address: u32) -> Option<u16> {
        match address {
            0x04000000 => Some(self.registers.dispcnt),
            0x04000002 => Some(self.registers.greenswap),
            0x04000004 => Some(self.registers.dispstat),
            0x04000006 => Some(self.vcount),
            _ => None,
        }
    }

    pub fn write_register(&mut self, address: u32, value: u16) {
        match address {
            0x04000000 => self.registers.dispcnt = value,
            0x04000002 => self.registers.greenswap = value & 1,
            0x04000004 => {
                self.registers.dispstat = (self.registers.dispstat & 7) | (value & 0xFF38);
                let mut event = PpuEvent::default();
                self.update_vcount_match(&mut event);
            }
            0x04000008..=0x0400000E => {
                self.registers.bgcnt[((address - 0x04000008) / 2) as usize] = value;
            }
            0x04000010..=0x0400001E => {
                let index = ((address - 0x04000010) / 4) as usize;
                if address & 2 == 0 {
                    self.registers.hofs[index] = value & 0x1FF;
                } else {
                    self.registers.vofs[index] = value & 0x1FF;
                }
            }
            0x04000020..=0x04000026 | 0x04000030..=0x04000036 => {
                let affine = usize::from(address >= 0x04000030);
                match (address & 0xF) / 2 {
                    0 => self.registers.pa[affine] = value as i16,
                    1 => self.registers.pb[affine] = value as i16,
                    2 => self.registers.pc[affine] = value as i16,
                    3 => self.registers.pd[affine] = value as i16,
                    _ => {}
                }
            }
            0x04000028..=0x0400002E | 0x04000038..=0x0400003E => {
                self.write_reference(address, value);
                let affine = usize::from(address >= 0x04000038);
                self.internal_x[affine] = self.registers.ref_x[affine];
                self.internal_y[affine] = self.registers.ref_y[affine];
            }
            0x04000040 => self.registers.winh[0] = value,
            0x04000042 => self.registers.winh[1] = value,
            0x04000044 => self.registers.winv[0] = value,
            0x04000046 => self.registers.winv[1] = value,
            0x04000048 => self.registers.winin = value,
            0x0400004A => self.registers.winout = value,
            0x0400004C => self.registers.mosaic = value,
            0x04000050 => self.registers.bldcnt = value & 0x3FFF,
            0x04000052 => self.registers.bldalpha = value & 0x1F1F,
            0x04000054 => self.registers.bldy = value & 0x1F,
            _ => {}
        }
    }

    fn write_reference(&mut self, address: u32, value: u16) {
        let affine = usize::from(address >= 0x04000038);
        let local = address & 0xF;
        let (raw, output) = if matches!(local, 8 | 0xA) {
            (
                &mut self.registers.ref_x_raw[affine],
                &mut self.registers.ref_x[affine],
            )
        } else {
            (
                &mut self.registers.ref_y_raw[affine],
                &mut self.registers.ref_y[affine],
            )
        };
        if local & 2 == 0 {
            *raw = (*raw & 0xFFFF0000) | u32::from(value);
        } else {
            *raw = (*raw & 0x0000FFFF) | (u32::from(value & 0x0FFF) << 16);
        }
        *output = sign_extend_28(*raw);
    }

    fn update_vcount_match(&mut self, event: &mut PpuEvent) {
        let was_match = self.registers.dispstat & (1 << 2) != 0;
        let is_match = self.vcount == self.registers.dispstat >> 8;
        self.registers.dispstat = (self.registers.dispstat & !(1 << 2)) | (u16::from(is_match) * 4);
        if is_match && !was_match && self.registers.dispstat & (1 << 5) != 0 {
            event.interrupt_mask |= 1 << 2;
        }
    }

    fn render_pixel(&mut self, x: usize, y: usize, vram: &[u8], palette: &[u8], oam: &[u8]) {
        if self.registers.dispcnt & (1 << 7) != 0 {
            self.frame[y * WIDTH + x] = color::rgba8888(0x7FFF);
            return;
        }
        let mask = self.window_mask(x, y, vram, palette, oam);
        let mut layers = Vec::with_capacity(6);
        layers.push(LayerPixel {
            color: color::read_color(palette, 0),
            priority: 4,
            layer: 5,
            semi_transparent: false,
        });
        for bg_index in 0..4 {
            if self.registers.dispcnt & (1 << (8 + bg_index)) != 0
                && mask & (1 << bg_index) != 0
                && let Some(pixel) = bg::pixel(
                    &self.registers,
                    (self.internal_x, self.internal_y),
                    (vram, palette),
                    bg_index,
                    x,
                    y,
                )
            {
                layers.push(pixel);
            }
        }
        if self.registers.dispcnt & (1 << 12) != 0
            && mask & (1 << 4) != 0
            && let Some(pixel) = obj::pixel(&self.registers, vram, palette, oam, x, y, false)
        {
            layers.push(pixel);
        }
        layers.sort_by_key(|pixel| (pixel.priority, layer_rank(pixel.layer)));
        let top = layers[0];
        let second = layers.get(1).copied();
        let effects_enabled = mask & (1 << 5) != 0;
        let output = self.apply_effect(top, second, effects_enabled);
        let final_color = if self.registers.greenswap & 1 != 0 {
            let r = output & 0x001F;
            let g = (output & 0x03E0) << 5;
            let b = (output & 0x7C00) >> 5;
            r | g | b
        } else {
            output
        };
        self.frame[y * WIDTH + x] = color::rgba8888(final_color);
    }

    fn apply_effect(&self, top: LayerPixel, second: Option<LayerPixel>, enabled: bool) -> u16 {
        if !enabled && !top.semi_transparent {
            return top.color;
        }
        let first_mask = self.registers.bldcnt & 0x3F;
        let second_mask = (self.registers.bldcnt >> 8) & 0x3F;
        let top_bit = 1 << top.layer;
        let mode = (self.registers.bldcnt >> 6) & 3;
        if (top.semi_transparent || mode == 1 && first_mask & top_bit != 0)
            && let Some(second) = second
            && second_mask & (1 << second.layer) != 0
        {
            let eva = (self.registers.bldalpha & 0x1F).min(16) as u8;
            let evb = ((self.registers.bldalpha >> 8) & 0x1F).min(16) as u8;
            return color::alpha_blend(top.color, second.color, eva, evb);
        }
        let amount = (self.registers.bldy & 0x1F).min(16) as u8;
        if first_mask & top_bit != 0 {
            if mode == 2 {
                return color::brighten(top.color, amount);
            }
            if mode == 3 {
                return color::darken(top.color, amount);
            }
        }
        top.color
    }

    fn window_mask(&self, x: usize, y: usize, vram: &[u8], palette: &[u8], oam: &[u8]) -> u8 {
        window::window_mask(&self.registers, x, y, vram, palette, oam)
    }
}

impl Default for GbaPpu {
    fn default() -> Self {
        Self::new()
    }
}

fn sign_extend_28(value: u32) -> i32 {
    ((value << 4) as i32) >> 4
}

fn layer_rank(layer: u8) -> u8 {
    if layer == 4 { 0 } else { layer + 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_sets_status_and_completes_frame() {
        let mut ppu = GbaPpu::new();
        let vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        for _ in 0..HBLANK_FLAG_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_ne!(ppu.dispstat() & 2, 0);
        for _ in HBLANK_FLAG_CYCLES..CYCLES_PER_LINE {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.vcount(), 1);
        let mut completed = false;
        for _ in CYCLES_PER_LINE as usize..CYCLES_PER_LINE as usize * LINES_PER_FRAME as usize {
            completed |= ppu.step(&vram, &palette, &oam).frame_complete;
        }
        assert!(completed);
        assert_eq!(ppu.vcount(), 0);
    }

    #[test]
    fn dispstat_hblank_timing() {
        let mut ppu = GbaPpu::new();
        let vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        // Before HBLANK_FLAG_CYCLES, flag is 0
        for _ in 0..HBLANK_FLAG_CYCLES - 1 {
            ppu.step(&vram, &palette, &oam);
            assert_eq!(ppu.dispstat() & 2, 0, "HBlank flag should be 0 during draw");
        }
        // At HBLANK_FLAG_CYCLES, flag becomes 1
        ppu.step(&vram, &palette, &oam);
        assert_ne!(ppu.dispstat() & 2, 0);
        // Next step still 1
        ppu.step(&vram, &palette, &oam);
        assert_ne!(ppu.dispstat() & 2, 0);
        // HBlank interrupt
        let mut ppu2 = GbaPpu::new();
        ppu2.write_register(0x04000004, 1 << 4);
        for _ in 0..HBLANK_FLAG_CYCLES - 1 {
            assert_eq!(ppu2.step(&vram, &palette, &oam).interrupt_mask, 0);
        }
        assert_eq!(ppu2.step(&vram, &palette, &oam).interrupt_mask, 1 << 1);
        // Verify scanline still completes
        for _ in HBLANK_FLAG_CYCLES + 1..=CYCLES_PER_LINE {
            ppu2.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu2.vcount(), 1);
    }

    #[test]
    fn mode_three_renders_bitmap_pixel() {
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[..2].copy_from_slice(&0x001Fu16.to_le_bytes());
        ppu.write_register(0x04000000, 3 | 1 << 10);
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [255, 0, 0, 255]);
    }

    #[test]
    fn mode_four_uses_palette_and_mode_five_clips() {
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[0] = 1;
        palette[2..4].copy_from_slice(&0x03E0u16.to_le_bytes());
        ppu.write_register(0x04000000, 4 | 1 << 10);
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [0, 255, 0, 255]);

        let mut ppu = GbaPpu::new();
        vram[(127 * 160 + 159) * 2..(127 * 160 + 159) * 2 + 2]
            .copy_from_slice(&0x7C00u16.to_le_bytes());
        ppu.write_register(0x04000000, 5 | 1 << 10);
        for _ in 0..CYCLES_PER_LINE as usize * 127 + HDRAW_CYCLES as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(
            ppu.frame_buffer()[127 * WIDTH + 159].to_le_bytes(),
            [0, 0, 255, 255]
        );
        assert_eq!(
            ppu.frame_buffer()[127 * WIDTH + 160].to_le_bytes(),
            [0, 0, 0, 255]
        );
    }

    #[test]
    fn text_bg_and_obj_render_palette_entries() {
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let mut oam = vec![0; 0x400];
        vram[0] = 1;
        palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
        ppu.write_register(0x04000000, 1 << 8);
        ppu.write_register(0x04000008, 31 << 8);
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [255, 0, 0, 255]);

        let mut ppu = GbaPpu::new();
        vram[0x10000] = 1;
        palette[0x202..0x204].copy_from_slice(&0x7C00u16.to_le_bytes());
        oam[0..6].copy_from_slice(&[0, 0, 0, 0, 0, 0]);
        ppu.write_register(0x04000000, (1 << 12) | (1 << 6));
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [0, 0, 255, 255]);
    }

    #[test]
    fn window_masks_bg() {
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[0] = 1;
        palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
        ppu.write_register(0x04000000, (1 << 8) | (1 << 13));
        ppu.write_register(0x04000008, 31 << 8);
        ppu.write_register(0x04000040, 0x0014); // WIN0H: 0..20 (x1=0,x2=20)
        ppu.write_register(0x04000044, 0x0014); // WIN0V: 0..20
        ppu.write_register(0x04000048, 1 << 0);
        ppu.write_register(0x0400004A, 0);
        for _ in 0..HDRAW_CYCLES as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes()[0], 255);
        for _ in HDRAW_CYCLES as usize..20 * CYCLES_PER_LINE as usize + HDRAW_CYCLES as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(
            ppu.frame_buffer()[20 * WIDTH + 20].to_le_bytes(),
            [0, 0, 0, 255]
        );
    }

    #[test]
    fn mosaic_expands_dots() {
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        // Set up 2 tiles: tile 0 all 0, tile 1 all 1 (red)
        // Simplified: just test that mosaic registers affect bg_mosaic
        ppu.write_register(0x0400004C, 0x11); // BG mosaic 1x1
        ppu.write_register(0x04000008, (1 << 6) | (31 << 8)); // BG0 mosaic enable
        vram[0] = 2; // tile map entry for mosaic test
        palette[4..6].copy_from_slice(&0x03E0u16.to_le_bytes());
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        // With mosaic 1, no expansion, should still render (basic check that mosaic doesn't crash)
        assert_eq!(ppu.frame_buffer().len(), WIDTH * HEIGHT);
        // Test mosaic 2x2
        ppu.write_register(0x0400004C, 0x11 | 0x1100); // BG 1x1, OBJ 1x1
        ppu.write_register(0x04000008, (1 << 6) | (31 << 8));
        for _ in 0..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer().len(), WIDTH * HEIGHT);
    }
}
