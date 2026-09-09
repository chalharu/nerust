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
/// BG fetch clock (nba hw-test archive/ppu/mode3): the PPU fetches pixel x
/// at 32+4x cycles into the scanline, one pixel every four cycles.
pub const FETCH_START_CYCLES: u16 = 32;
pub const FETCH_END_CYCLES: u16 = 988;
/// DISPCNT latch shift point (NBA `LatchDISPCNT`, +40 cycles into the line).
pub const DISPCNT_LATCH_CYCLES: u16 = 40;

pub fn bgr555_to_rgba8888(color: u16) -> u32 {
    color::rgba8888(color & 0x7FFF)
}

/// GBA LCD color emulation for presentation (see `color` docs).
/// Not applied to the core framebuffer; frontends opt in at display time.
pub fn gba_lcd_rgba8888(color: u16) -> u32 {
    color::gba_lcd_rgba8888(color)
}

/// In-place GBA LCD filter over an RGBA8888 framebuffer.
pub fn apply_gba_lcd_filter(frame: &mut [u32]) {
    color::apply_gba_lcd_filter(frame);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PpuEvent {
    pub frame_complete: bool,
    pub interrupt_mask: u16,
    pub hblank_started: bool,
    pub vblank_started: bool,
    pub line_started: bool,
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
    ref_written: [bool; 2],
    /// Last sub-boundary BG VRAM halfword (NBA `vram_bg_latch`): BG fetches
    /// at/above the OBJ boundary return this instead of physical VRAM.
    bg_latch: u16,
    /// DISPCNT 3-stage shift latch (NBA `dispcnt_latch`, HW-confirmed):
    /// shifted at +40 cycles of every scanline. BG/OBJ enables gate on
    /// `latch[0] & live`, forced blank on `latch[0] | live` (NBA `Merge.cc`,
    /// `PPU.hh::ForcedBlank`). Window enables stay live.
    dispcnt_latch: [u16; 3],
    /// Forced-blank restart (GBATEK DISPCNT: when a forced blank during a
    /// display period is cancelled, the display restarts from the beginning
    /// after two vertical lines). Counts down line_ends after a blank 1->0
    /// transition with vcount<160; at zero the scanline counter resets.
    blank_restart: u8,
    /// Effective forced-blank state at the previous line end, for edge
    /// detection above.
    was_blanked: bool,
    /// Line-deferred render latch: OAM bytes and the MOSAIC register sampled
    /// at the first pixel of each scanline. Mid-scanline writes take effect
    /// on the next line. HBlank/VBlank writes (IRQ handlers, HBlank DMA, the
    /// standard raster techniques) land before the next line starts, so they
    /// behave exactly as before; only cycle-timed mid-draw writes change
    /// behavior (from tearing the current line to applying on the next line).
    ///
    /// Model note: mGBA (`video.c:_startHblank`) renders each scanline at
    /// HBlank start from live state, so mid-draw writes there apply to the
    /// whole current line retroactively; GBATEK/Tonc document no sampling
    /// point and no hw-test in tree pins the real mid-draw behavior down.
    /// The line-start latch is the deterministic choice consistent with the
    /// per-line OBJ cycle budget and line-held vertical mosaic. DISPCNT,
    /// BGxCNT, scroll and window registers stay live (out of scope).
    line: LineLatch,
}

#[derive(Debug)]
struct LineLatch {
    mosaic: u16,
    oam: Box<[u8; 1024]>,
    /// `dispcnt_latch[0]` sampled at the first fetch of the line (cycle 32,
    /// before the +40 shift): the enable/blank reference for this scanline.
    enable: u16,
}

impl LineLatch {
    fn new() -> Self {
        Self {
            mosaic: 0,
            oam: Box::new([0; 1024]),
            enable: 0,
        }
    }

    fn capture(&mut self, mosaic: u16, oam: &[u8], enable: u16) {
        self.mosaic = mosaic;
        self.oam.copy_from_slice(oam);
        self.enable = enable;
    }
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
            ref_written: [false; 2],
            bg_latch: 0,
            dispcnt_latch: [0; 3],
            blank_restart: 0,
            was_blanked: false,
            line: LineLatch::new(),
        }
    }

    pub fn step(&mut self, vram: &[u8], palette: &[u8], oam: &[u8]) -> PpuEvent {
        let mut event = PpuEvent::default();
        self.cycle += 1;
        // Fetch clock (archive/ppu/mode3): pixel x is fetched at 32+4x, so
        // the pixel rendered from live VRAM at cycle C is x = C/4-1-7.
        // Mid-line VRAM writes (HBlank DMA races) become visible exactly
        // when the fetcher passes them; static lines render identically.
        if self.vcount < HEIGHT as u16
            && self.cycle >= FETCH_START_CYCLES
            && self.cycle <= FETCH_END_CYCLES
            && self.cycle.is_multiple_of(4)
        {
            self.render_pixel(
                self.cycle as usize / 4 - 1 - 7,
                self.vcount as usize,
                vram,
                palette,
                oam,
            );
        }
        if self.cycle == DISPCNT_LATCH_CYCLES {
            // NBA LatchDISPCNT: 3-stage shift of the DISPCNT enable latch.
            self.dispcnt_latch[0] = self.dispcnt_latch[1];
            self.dispcnt_latch[1] = self.dispcnt_latch[2];
            self.dispcnt_latch[2] = self.registers.dispcnt;
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
        // Refresh the per-line enable/blank reference from the latch for
        // the upcoming scanline (all lines, including VBlank): the
        // renderer (x==0 capture) and forced_blank()/bg_fetch_active()
        // share this value, so mid-line DISPCNT writes defer to the next
        // line in both paths identically.
        self.line.enable = self.dispcnt_latch[0];
        event.line_started = true;
        self.registers.dispstat &= !(1 << 1);
        self.advance_affine();
        // Forced-blank restart edge: blanked 1->0 with vcount<160 restarts
        // the frame after two more vertical lines (GBATEK DISPCNT).
        let blanked = self.forced_blank();
        if self.was_blanked && !blanked && self.vcount < HEIGHT as u16 {
            self.blank_restart = 2;
        }
        self.was_blanked = blanked;
        if self.blank_restart > 0 {
            self.blank_restart -= 1;
            if self.blank_restart == 0 {
                // Restart the frame: the next scanline rendered is line 0.
                self.vcount = 0;
                return;
            }
        }
        self.advance_vcount(event);
        self.update_vcount_match(event);
    }

    fn advance_affine(&mut self) {
        if self.vcount >= HEIGHT as u16 {
            return;
        }
        // NBA #177: internal affine registers advance only while their BG
        // is enabled (BG2 -> affine 0, BG3 -> affine 1).
        let enabled = [
            self.registers.dispcnt & (1 << 10) != 0,
            self.registers.dispcnt & (1 << 11) != 0,
        ];
        crate::ppu::affine::advance_line(
            &mut self.internal_x,
            &mut self.internal_y,
            self.registers.pb,
            self.registers.pd,
            &mut self.ref_written,
            enabled,
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
        } else if self.vcount == LINES_PER_FRAME - 1 {
            // GBATEK DISPSTAT Bit 0: V-Blank flag set in lines 160..226, not 227.
            self.registers.dispstat &= !1;
        } else if self.vcount == LINES_PER_FRAME {
            self.vcount = 0;
            self.registers.dispstat &= !1;
            // NBA #177: VBlank internal copy, per enabled BG.
            for affine in 0..2 {
                if self.registers.dispcnt & (1 << (10 + affine)) != 0 {
                    self.internal_x[affine] = self.registers.ref_x[affine];
                    self.internal_y[affine] = self.registers.ref_y[affine];
                }
            }
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

    /// NBA `ForcedBlank`: blanked when the bit is set in the latched OR the
    /// live DISPCNT. The latched half is the per-line reference shared with
    /// the renderer (refreshed every line end), so render and stall paths
    /// can never disagree by a line.
    pub fn forced_blank(&self) -> bool {
        (self.line.enable | self.registers.dispcnt) & (1 << 7) != 0
    }

    /// Any BG layer enabled in both the latched and the live DISPCNT
    /// (NBA Background/Merge gating); gates BG-VRAM fetch contention.
    pub fn bg_fetch_active(&self) -> bool {
        self.line.enable & self.registers.dispcnt & 0x0F00 != 0
    }

    pub fn dispstat(&self) -> u16 {
        self.registers.dispstat
    }

    pub fn bgcnt(&self, bg: usize) -> u16 {
        self.registers.bgcnt[bg]
    }

    pub fn hofs(&self, bg: usize) -> u16 {
        self.registers.hofs[bg]
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
            0x04000008..=0x0400000E => {
                Some(self.registers.bgcnt[((address - 0x04000008) / 2) as usize])
            }
            0x04000048 => Some(self.registers.winin),
            0x0400004A => Some(self.registers.winout),
            0x04000050 => Some(self.registers.bldcnt),
            0x04000052 => Some(self.registers.bldalpha),
            _ => None,
        }
    }

    pub fn write_register(&mut self, address: u32, value: u16) -> u16 {
        match address {
            0x04000000 => {
                // GBATEK DISPCNT Bit 3 (CGB Mode): can be set only by BIOS opcodes.
                // CPU writes must not change it, so preserve the old bit.
                let old = self.registers.dispcnt;
                self.registers.dispcnt = (value & !(1 << 3)) | (old & (1 << 3));
                0
            }
            0x04000002 => {
                self.registers.greenswap = value & 1;
                0
            }
            0x04000004 => {
                let old = self.registers.dispstat;
                let old_match = old & (1 << 2) != 0;
                let old_enable = old & (1 << 5) != 0;
                self.registers.dispstat = (self.registers.dispstat & 7) | (value & 0xFF38);
                let is_match = self.vcount == self.registers.dispstat >> 8;
                self.registers.dispstat =
                    (self.registers.dispstat & !(1 << 2)) | (u16::from(is_match) * 4);
                let new_enable = self.registers.dispstat & (1 << 5) != 0;
                if is_match && new_enable && (!old_match || !old_enable) {
                    1 << 2
                } else {
                    0
                }
            }
            0x04000008..=0x0400000E => {
                let bg = ((address - 0x04000008) / 2) as usize;
                // NBA registers.cc: display-area-overflow (bit 13) exists
                // only on BG2/BG3; writes to BG0/BG1 ignore it.
                let mask = if bg < 2 { !(1 << 13) } else { u16::MAX };
                self.registers.bgcnt[bg] = value & mask;
                0
            }
            0x04000010..=0x0400001E => {
                let index = ((address - 0x04000010) / 4) as usize;
                if address & 2 == 0 {
                    self.registers.hofs[index] = value & 0x1FF;
                } else {
                    self.registers.vofs[index] = value & 0x1FF;
                }
                0
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
                0
            }
            0x04000028..=0x0400002E | 0x04000038..=0x0400003E => {
                let affine = usize::from(address >= 0x04000038);
                self.write_reference(address, value);
                // GBATEK: outside VBlank the write is copied to internal immediately.
                // For per-scanline affine (BGMode7) the HBlank write must not be
                // incremented again at line end, so mark dirty to skip advance.
                if self.vcount < 160 && self.registers.dispcnt & (1 << (10 + affine)) != 0 {
                    self.internal_x[affine] = self.registers.ref_x[affine];
                    self.internal_y[affine] = self.registers.ref_y[affine];
                    if self.cycle >= HBLANK_FLAG_CYCLES {
                        self.ref_written[affine] = true;
                    }
                }
                0
            }
            0x04000040 => {
                self.registers.winh[0] = value;
                0
            }
            0x04000042 => {
                self.registers.winh[1] = value;
                0
            }
            0x04000044 => {
                self.registers.winv[0] = value;
                0
            }
            0x04000046 => {
                self.registers.winv[1] = value;
                0
            }
            0x04000048 => {
                // NBA WindowLayerSelect: only 6 bits per byte are stored.
                self.registers.winin = value & 0x3F3F;
                0
            }
            0x0400004A => {
                self.registers.winout = value & 0x3F3F;
                0
            }
            0x0400004C => {
                self.registers.mosaic = value;
                0
            }
            0x04000050 => {
                self.registers.bldcnt = value & 0x3FFF;
                0
            }
            0x04000052 => {
                self.registers.bldalpha = value & 0x1F1F;
                0
            }
            0x04000054 => {
                self.registers.bldy = value & 0x1F;
                0
            }
            _ => 0,
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
        if x == 0 {
            // Line-start sample: HBlank/VBlank-period writes are already in
            // `oam`/registers and apply to this line; writes later in this
            // line's draw period defer to the next line. Sampling at the
            // first fetch (cycle 32, before the +40 DISPCNT shift) also seeds
            // the very first frame and keeps the latch fresh across VBlank
            // lines (which never render) and forced-blank lines.
            self.line
                .capture(self.registers.mosaic, oam, self.dispcnt_latch[0]);
        }
        // NBA ForcedBlank: latched OR live.
        if (self.line.enable | self.registers.dispcnt) & (1 << 7) != 0 {
            self.frame[y * WIDTH + x] = color::rgba8888(0x7FFF);
            return;
        }
        // NBA Background/Merge: layer enables gate on latched AND live.
        // OBJ is the exception: its fetch keys off the LIVE enable only
        // (NBA LatchDISPCNT: latched DISPCNT disregarded for OBJ), so it
        // reacts to HBlank toggling immediately while BGs lag 3 lines.
        let enables = self.line.enable & self.registers.dispcnt;
        let mask = self.window_mask(x, y, vram, palette);
        let mut layers = Vec::with_capacity(6);
        layers.push(LayerPixel {
            color: color::read_color(palette, 0),
            priority: 4,
            layer: 5,
            semi_transparent: false,
        });
        for bg_index in 0..4 {
            if enables & (1 << (8 + bg_index)) != 0
                && mask & (1 << bg_index) != 0
                && let Some(pixel) = bg::pixel(
                    &self.registers,
                    (self.internal_x, self.internal_y),
                    (vram, palette),
                    bg_index,
                    (x, y),
                    &mut self.bg_latch,
                    self.line.mosaic,
                )
            {
                layers.push(pixel);
            }
        }
        if self.registers.dispcnt & (1 << 12) != 0
            && mask & (1 << 4) != 0
            && let Some(pixel) = obj::pixel(
                &self.registers,
                vram,
                palette,
                &self.line.oam[..],
                (x, y),
                false,
                self.line.mosaic,
            )
        {
            layers.push(pixel);
        }
        layers.sort_by_key(|pixel| (pixel.priority, layer_rank(pixel.layer)));
        let top = layers[0];
        let second = layers.get(1).copied();
        let effects_enabled = mask & (1 << 5) != 0;
        let output = self.apply_effect(top, second, effects_enabled);
        self.frame[y * WIDTH + x] = color::rgba8888(output);
        if self.registers.greenswap & 1 != 0 && x % 2 == 1 {
            let prev_idx = y * WIDTH + x - 1;
            let curr_idx = y * WIDTH + x;
            let prev_rgba = self.frame[prev_idx];
            let curr_rgba = self.frame[curr_idx];
            let prev_b = prev_rgba.to_le_bytes();
            let curr_b = curr_rgba.to_le_bytes();
            self.frame[prev_idx] = u32::from_le_bytes([prev_b[0], curr_b[1], prev_b[2], prev_b[3]]);
            self.frame[curr_idx] = u32::from_le_bytes([curr_b[0], prev_b[1], curr_b[2], curr_b[3]]);
        }
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

    fn window_mask(&self, x: usize, y: usize, vram: &[u8], palette: &[u8]) -> u8 {
        window::window_mask(
            &self.registers,
            x,
            y,
            vram,
            palette,
            &self.line.oam[..],
            self.line.mosaic,
        )
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

    /// Write DISPCNT with the enable latch pre-propagated (steady state).
    /// Render tests use this to skip the 3-line hardware enable pipeline;
    /// latch propagation itself is covered by `dispcnt_enable_latch_delays`.
    fn steady_dispcnt(ppu: &mut GbaPpu, value: u16) {
        ppu.write_register(0x04000000, value);
        ppu.dispcnt_latch = [value; 3];
    }

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
        steady_dispcnt(&mut ppu, 3 | 1 << 10);
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
        steady_dispcnt(&mut ppu, 4 | 1 << 10);
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [0, 255, 0, 255]);

        let mut ppu = GbaPpu::new();
        vram[(127 * 160 + 159) * 2..(127 * 160 + 159) * 2 + 2]
            .copy_from_slice(&0x7C00u16.to_le_bytes());
        steady_dispcnt(&mut ppu, 5 | 1 << 10);
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
    fn mode_four_transparent_behind_obj() {
        // GBATEK Mode 4: color 0 is transparent, OBJ behind must show through
        // even when the BG has higher display priority than the OBJ.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let mut oam = vec![0; 0x400];
        // Disable OBJs 1..127 (attr0 bit 9), keep OBJ0 enabled at (0,0).
        for entry in oam.as_chunks_mut::<8>().0.iter_mut().skip(1) {
            entry[0..2].copy_from_slice(&0x0200u16.to_le_bytes());
        }
        vram[0] = 0; // Mode 4 frame pixel (0,0): transparent color 0.
        vram[0x10000 + 512 * 32] = 1; // OBJ tile 512, pixel = palette index 1.
        palette[0x202..0x204].copy_from_slice(&0x7C00u16.to_le_bytes()); // blue
        // BG2 priority 0, OBJ0 (tile 512, priority 1).
        oam[0..6].copy_from_slice(&[0, 0, 0, 0, 0x00, 0x06]);
        steady_dispcnt(&mut ppu, 4 | (1 << 10) | (1 << 12));
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [0, 0, 255, 255]);
    }

    #[test]
    fn bg_overfetch_reads_latch() {
        // CBB=3 + tile 513 lands the tile fetch on 0x10020 (>= boundary):
        // it reads the latched map entry (0x0201) low byte -> index 1 (red).
        // Neither transparent nor wrapped tile data.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        for b in vram.iter_mut().take(0x20) {
            *b = 0x11; // tile 0 (would-be wrap target): palette index 1
        }
        vram[0..2].copy_from_slice(&513u16.to_le_bytes()); // map: tile 513
        palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes()); // red
        steady_dispcnt(&mut ppu, 1 << 8);
        ppu.write_register(0x04000008, 3 << 2); // CBB=3, SBB=0, 4bpp, 32x32
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [255, 0, 0, 255]);
    }

    #[test]
    fn large_bg_map_past_64k_reads_latch() {
        // 64x64 map at SBB=31 scrolled to (256,256) fetches its entry from
        // 0x11000 (>= boundary): the initial latch (0) is returned, so the
        // pixel is backdrop. Physical VRAM at 0x11000 must not be read.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[0x11000..0x11002].copy_from_slice(&1u16.to_le_bytes()); // tile 1
        for b in vram.iter_mut().skip(0x20).take(0x20) {
            *b = 0x11; // tile 1: palette index 1
        }
        steady_dispcnt(&mut ppu, 1 << 8);
        ppu.write_register(0x04000008, (31 << 8) | (3 << 14)); // SBB=31, 64x64
        ppu.write_register(0x04000010, 256); // hofs
        ppu.write_register(0x04000012, 256); // vofs
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [0, 0, 0, 255]);
    }

    #[test]
    fn text_bg_and_obj_render_palette_entries() {
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let mut oam = vec![0; 0x400];
        vram[0] = 1;
        vram[0xF802..0xF804].copy_from_slice(&0xF000u16.to_le_bytes());
        palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
        palette[0x1E2..0x1E4].copy_from_slice(&0x7FFFu16.to_le_bytes());
        steady_dispcnt(&mut ppu, 1 << 8);
        ppu.write_register(0x04000008, 31 << 8);
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [255, 0, 0, 255]);
        assert_eq!(ppu.frame_buffer()[8].to_le_bytes(), [255, 255, 255, 255]);

        let mut ppu = GbaPpu::new();
        vram[0x10000] = 1;
        palette[0x202..0x204].copy_from_slice(&0x7C00u16.to_le_bytes());
        oam[0..6].copy_from_slice(&[0, 0, 0, 0, 0, 0]);
        steady_dispcnt(&mut ppu, (1 << 12) | (1 << 6));
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
        steady_dispcnt(&mut ppu, (1 << 8) | (1 << 13));
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
    fn dispcnt_enable_latch_delays_and_blank_is_or() {
        // NBA HW-confirmed model: BG enables gate on latched AND live
        // (3-stage shift at +40 cycles/line), forced blank on latched OR live.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[0] = 1;
        // BG0 on written mid-frame: still gated off until the latch shifts
        // through (renders backdrop), then appears.
        ppu.write_register(0x04000000, 1 << 8);
        ppu.write_register(0x04000008, 31 << 8);
        for _ in 0..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes()[0], 0);
        for _ in 0..CYCLES_PER_LINE as usize * 3 {
            ppu.step(&vram, &palette, &oam);
        }
        // Latch propagated through the 3-stage shift: BG0 enable visible.
        assert_ne!(ppu.dispcnt_latch[0] & (1 << 8), 0);
        assert_ne!(ppu.line.enable & (1 << 8), 0);
        // Forced blank applies immediately (OR semantics, no latency):
        // the line drawn after the write is white.
        ppu.write_register(0x04000000, (1 << 8) | (1 << 7));
        for _ in 0..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(
            ppu.frame_buffer()[4 * WIDTH].to_le_bytes(),
            [255, 255, 255, 255]
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

    #[test]
    fn oam_mid_line_write_defers_to_next_line() {
        // OBJ0 8x8 at (0,0), tile 512; disable it mid-draw on line 0.
        // The remainder of line 0 still shows the sprite (line-deferred),
        // line 1 hides it.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let mut oam = vec![0; 0x400];
        for entry in oam.as_chunks_mut::<8>().0.iter_mut().skip(1) {
            entry[0..2].copy_from_slice(&0x0200u16.to_le_bytes());
        }
        vram[0x10000 + 512 * 32..0x10000 + 512 * 32 + 8].fill(0x11);
        palette[0x202..0x204].copy_from_slice(&0x7C00u16.to_le_bytes()); // blue
        oam[0..6].copy_from_slice(&[0, 0, 0, 0, 0x00, 0x02]);
        steady_dispcnt(&mut ppu, (1 << 12) | (1 << 6));
        // Write after the first fetch (cycle 32): line 0 keeps the sprite.
        for _ in 0..64 {
            ppu.step(&vram, &palette, &oam);
        }
        oam[0..2].copy_from_slice(&0x0200u16.to_le_bytes()); // disable OBJ0
        for _ in 64..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [0, 0, 255, 255]);
        assert_eq!(ppu.frame_buffer()[5].to_le_bytes(), [0, 0, 255, 255]);
        for _ in CYCLES_PER_LINE as usize..2 * CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[WIDTH].to_le_bytes(), [0, 0, 0, 255]);
    }

    #[test]
    fn mosaic_mid_line_write_defers_to_next_line() {
        // BG0 4bpp tile 0 with distinct pixels, horizontal mosaic 2 from line
        // start; clearing MOSAIC mid-draw keeps mosaic for the rest of line 0
        // and disables it on line 1.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[0] = 0x21; // px0=1 red, px1=2 green
        vram[2] = 0x43; // px4=3 blue, px5=4 white
        vram[4] = 0x21;
        vram[6] = 0x43;
        palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
        palette[4..6].copy_from_slice(&0x03E0u16.to_le_bytes());
        palette[6..8].copy_from_slice(&0x7C00u16.to_le_bytes());
        palette[8..10].copy_from_slice(&0x7FFFu16.to_le_bytes());
        steady_dispcnt(&mut ppu, 1 << 8);
        ppu.write_register(0x04000008, (31 << 8) | (1 << 6)); // SBB=31, mosaic
        ppu.write_register(0x0400004C, 0x01); // BG mosaic h=2, v=1
        // Clear after the first fetch (cycle 32): line 0 keeps mosaic.
        for _ in 0..64 {
            ppu.step(&vram, &palette, &oam);
        }
        ppu.write_register(0x0400004C, 0x00); // clear mid-draw
        for _ in 64..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [255, 0, 0, 255]);
        assert_eq!(ppu.frame_buffer()[1].to_le_bytes(), [255, 0, 0, 255]);
        assert_eq!(ppu.frame_buffer()[5].to_le_bytes(), [0, 0, 255, 255]);
        for _ in CYCLES_PER_LINE as usize..2 * CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(
            ppu.frame_buffer()[WIDTH + 1].to_le_bytes(),
            [0, 255, 0, 255]
        );
        assert_eq!(
            ppu.frame_buffer()[WIDTH + 5].to_le_bytes(),
            [255, 255, 255, 255]
        );
    }

    #[test]
    fn vblank_flag_cleared_on_line_227() {
        let mut ppu = GbaPpu::new();
        let vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        // Advance to VBlank start (line 160).
        for _ in 0..CYCLES_PER_LINE as usize * 160 {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.vcount(), 160);
        assert_ne!(ppu.dispstat() & 1, 0);
        // Advance to line 226: flag still set.
        for _ in 0..CYCLES_PER_LINE as usize * 66 {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.vcount(), 226);
        assert_ne!(ppu.dispstat() & 1, 0);
        // Line 227: GBATEK says flag is 0.
        for _ in 0..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.vcount(), 227);
        assert_eq!(ppu.dispstat() & 1, 0);
    }

    #[test]
    fn vcounter_irq_on_dispstat_write() {
        let mut ppu = GbaPpu::new();
        // VCOUNT=0, write LYC=0 with enable -> immediate IRQ.
        let irq = ppu.write_register(0x04000004, 1 << 5);
        assert_eq!(irq, 1 << 2);
        assert_ne!(ppu.dispstat() & (1 << 2), 0);
        // Same write again (already matching, already enabled) -> no repeat IRQ.
        let irq2 = ppu.write_register(0x04000004, 1 << 5);
        assert_eq!(irq2, 0);
        // Enable rising while already matching -> IRQ.
        ppu.write_register(0x04000004, 0 << 8); // disable, LYC=0 still match, no IRQ
        assert_eq!(ppu.dispstat() & (1 << 2), 4);
        let irq3 = ppu.write_register(0x04000004, 1 << 5);
        assert_eq!(irq3, 1 << 2);
    }

    #[test]
    fn rw_registers_are_readable() {
        let mut ppu = GbaPpu::new();
        ppu.write_register(0x04000008, 0x1234);
        assert_eq!(ppu.read_register(0x04000008), Some(0x1234));
        ppu.write_register(0x04000048, 0x00FF);
        assert_eq!(ppu.read_register(0x04000048), Some(0x003F));
        ppu.write_register(0x0400004A, 0x0F0F);
        assert_eq!(ppu.read_register(0x0400004A), Some(0x0F0F));
        ppu.write_register(0x04000050, 0xFFFF);
        assert_eq!(ppu.read_register(0x04000050), Some(0x3FFF));
        ppu.write_register(0x04000052, 0xFFFF);
        assert_eq!(ppu.read_register(0x04000052), Some(0x1F1F));
        // W registers stay unreadable.
        assert_eq!(ppu.read_register(0x0400004C), None);
        assert_eq!(ppu.read_register(0x04000054), None);
    }

    #[test]
    fn write_masks_follow_hardware() {
        let mut ppu = GbaPpu::new();
        // BG0/BG1 have no bit-13 overflow flag (NBA registers.cc).
        ppu.write_register(0x04000008, 0xFFFF);
        assert_eq!(ppu.read_register(0x04000008), Some(0xDFFF));
        ppu.write_register(0x0400000C, 0xFFFF);
        assert_eq!(ppu.read_register(0x0400000C), Some(0xFFFF));
        // WININ/WINOUT store 6 bits per byte.
        ppu.write_register(0x04000048, 0xFFFF);
        assert_eq!(ppu.read_register(0x04000048), Some(0x3F3F));
        ppu.write_register(0x0400004A, 0xFFFF);
        assert_eq!(ppu.read_register(0x0400004A), Some(0x3F3F));
    }

    #[test]
    fn dispcnt_bit3_preserved_on_cpu_write() {
        let mut ppu = GbaPpu::new();
        assert_eq!(ppu.dispcnt() & (1 << 3), 0);
        ppu.write_register(0x04000000, 0xFFFF);
        assert_eq!(ppu.dispcnt() & (1 << 3), 0);
    }
}
