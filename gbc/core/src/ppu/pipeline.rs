/// Per-pixel BG fetcher that exactly mirrors read_bg_pixel from the old
/// render_scanline. No FIFO — reads VRAM directly for each pixel.
/// This is a stepping stone toward the full FIFO pipeline.

#[derive(Debug, Clone)]
pub(super) struct Mode3Pipeline {
    pub(super) pixel_x: u8,
    pub(super) fine_scroll: u8,
    complete: bool,

    // Lached registers (updated by pending writes)
    pub(super) lcdc: u8,
    pub(super) scx: u8,
    pub(super) scy: u8,
    pub(super) wx: u8,
    pub(super) wy: u8,
    pub(super) bgp: u8,
    startup_dots: u8,

    // Window state
    pub(super) window_active: bool,
    pub(super) window_line: u8,

    // Pending register writes
    pending_writes: Vec<PendingWrite>,
}

#[derive(Debug, Clone, Copy)]
struct PendingWrite {
    pixel_x: u8,
    register: u16,
    value: u8,
}

impl Mode3Pipeline {
    pub(super) fn new(
        scx: u8, scy: u8, wx: u8, wy: u8, lcdc: u8, bgp: u8, cgb_mode: bool,
    ) -> Self {
        Self {
            pixel_x: 0,
            fine_scroll: scx & 7,
            complete: false,
            lcdc, scx, scy, wx, wy,
            bgp,
            startup_dots: if cgb_mode { 19 } else { 18 } + (scx & 7),
            window_active: false,
            window_line: 0,
            pending_writes: Vec::new(),
        }
    }

    pub(super) fn step(
        &mut self,
        vram: &[u8; 0x4000],
        bg_palette: &[u16; 32],
        cgb_mode: bool,
        cgb_game: bool,
        ly: u8,
    ) -> Option<u32> {
        if self.complete { return None; }
        self.apply_pending_writes();

        // Startup delay: pixel_x doesn't advance during pipeline fill
        if self.startup_dots > 0 {
            self.startup_dots -= 1;
            return None;
        }

        let scroll_x = self.scx.wrapping_add(self.pixel_x);
        let scroll_y = self.scy.wrapping_add(ly);
        let tile_col = (scroll_x / 8) as u16;
        let tile_row = (scroll_y / 8) as u16;
        let tile_x = scroll_x % 8;
        let tile_y = scroll_y % 8;

        // Read tile index from map
        let map_base: u16 = if self.lcdc & 0x08 != 0 { 0x9C00 } else { 0x9800 };
        let map_addr = map_base + tile_row * 32 + tile_col;
        let map_idx = (map_addr & 0x1FFF) as usize;
        let tile = vram[map_idx];

        // Read attribute byte (CGB only)
        let attr = if cgb_game { vram[0x2000 + map_idx] } else { 0 };
        let bank = if cgb_game { ((attr >> 3) & 0x01) as usize } else { 0 };
        let hflip = cgb_game && (attr & 0x20) != 0;
        let vflip = cgb_game && (attr & 0x40) != 0;

        // Apply vertical flip
        let eff_tile_y = if vflip { 7 - tile_y } else { tile_y };

        // Read pixel from tile data
        let signed = self.lcdc & 0x10 == 0;
        let tile_addr = if signed {
            let signed_idx = tile as i8 as i16;
            (0x9000u16).wrapping_add_signed(signed_idx.wrapping_mul(16))
        } else {
            0x8000u16 + tile as u16 * 16
        };

        let row_addr = tile_addr + eff_tile_y as u16 * 2;
        let row_idx = (row_addr & 0x1FFF) as usize;
        let low = vram[bank * 0x2000 + row_idx];
        let high = vram[bank * 0x2000 + row_idx + 1];

        // Apply horizontal flip (read bits in reverse order)
        let bit = if hflip { tile_x } else { 7 - tile_x };
        let color = ((low >> bit) & 1) | (((high >> bit) & 1) << 1);

        let pixel = if cgb_game {
            let pal = (attr & 0x07) as usize;
            Self::cgb_color(bg_palette[pal * 4 + color as usize])
        } else if cgb_mode {
            let shade = (self.bgp >> (color * 2)) & 0x03;
            Self::cgb_color(bg_palette[shade as usize])
        } else {
            let shade = (self.bgp >> (color * 2)) & 0x03;
            Self::shade_to_pixel(shade)
        };

        self.pixel_x += 1;
        if self.pixel_x >= 160 { self.complete = true; }
        Some(pixel)
    }

    pub(super) fn pixel_x(&self) -> u8 { self.pixel_x }
    pub(super) fn fine_scroll_x(&self) -> u8 { self.fine_scroll }
    pub(super) fn complete(&self) -> bool { self.complete }

    pub(super) fn queue_register_write(&mut self, register: u16, value: u8) -> u8 {
        // Tile-fetch-related registers: 6-dot delay for fetcher restart.
        // Palette registers: immediate (take effect at current pixel).
        let delay = match register {
            0xFF40 | 0xFF42 | 0xFF43 | 0xFF4A | 0xFF4B => 6,
            _ => 0, // 0xFF47 (BGP), 0xFF48 (OBP0), 0xFF49 (OBP1)
        };
        let apply_x = self.pixel_x.saturating_add(delay).min(159);
        self.pending_writes.push(PendingWrite { pixel_x: apply_x, register, value });
        apply_x
    }

    fn apply_pending_writes(&mut self) {
        self.pending_writes.retain(|w| {
            if w.pixel_x <= self.pixel_x {
                match w.register {
                    0xFF40 => self.lcdc = w.value,
                    0xFF42 => self.scy = w.value,
                    0xFF43 => self.scx = w.value,
                    0xFF47 => self.bgp = w.value,
                    0xFF4A => self.wy = w.value,
                    0xFF4B => self.wx = w.value,
                    _ => {}
                }
                false
            } else { true }
        });
    }

    fn cgb_color(color15: u16) -> u32 {
        let r = ((color15 >> 0) & 0x1F) as u32;
        let g = ((color15 >> 5) & 0x1F) as u32;
        let b = ((color15 >> 10) & 0x1F) as u32;
        let r8 = (r << 3) | (r >> 2);
        let g8 = (g << 3) | (g >> 2);
        let b8 = (b << 3) | (b >> 2);
        (r8 << 24) | (g8 << 16) | (b8 << 8) | 0xFF
    }

    fn shade_to_pixel(shade: u8) -> u32 {
        match shade {
            0 => 0xFF_FF_FF_FF, 1 => 0xAA_AA_AA_FF,
            2 => 0x55_55_55_FF, _ => 0x00_00_00_FF,
        }
    }
}
