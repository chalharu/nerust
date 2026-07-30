/// Per-pixel pipeline that handles BG AND window rendering.
/// Replaces render_scanline for both tile paths.

#[derive(Debug, Clone)]
pub(super) struct Mode3Pipeline {
    pub(super) pixel_x: u8,
    /// Total dots since mode 3 start (for fetch_pixel_x calculation)
    dot: u16,
    complete: bool,

    // Lached registers
    pub(super) lcdc: u8,
    pub(super) scx: u8,
    pub(super) scy: u8,
    pub(super) wx: u8,
    pub(super) wy: u8,
    pub(super) bgp: u8,
    startup_dots: u8,
    pub(super) fine_scroll: u8,

    // Window state
    pub(super) window_active: bool,
    pub(super) window_line: u8,
    window_pixel_count: u8,

    // Pending register writes
    pending_writes: Vec<PendingWrite>,
}

#[derive(Debug, Clone, Copy)]
struct PendingWrite { pixel_x: u8, register: u16, value: u8 }

impl Mode3Pipeline {
    pub(super) fn new(
        scx: u8, scy: u8, wx: u8, wy: u8, lcdc: u8, bgp: u8, cgb_mode: bool,
    ) -> Self {
        Self {
            pixel_x: 0, dot: 0, fine_scroll: scx & 7, complete: false,
            lcdc, scx, scy, wx, wy, bgp,
            startup_dots: if cgb_mode { 19 } else { 18 } + (scx & 7),
            window_active: false, window_line: 0, window_pixel_count: 0,
            pending_writes: Vec::new(),
        }
    }

    pub(super) fn step(
        &mut self, vram: &[u8; 0x4000], bg_palette: &[u16; 32],
        cgb_mode: bool, cgb_game: bool, ly: u8,
    ) -> Option<u32> {
        if self.complete { return None; }
        self.apply_pending_writes();

        if self.startup_dots > 0 { self.startup_dots -= 1; self.dot += 1; return None; }

        self.dot += 1;

        // Check window activation
        let window_x = self.wx as i16 - 7;
        if !self.window_active
            && self.lcdc & 0x20 != 0 && ly >= self.wy
            && window_x < 160
            && self.pixel_x as i16 >= window_x.max(0)
        {
            self.window_active = true;
            self.window_line = self.window_line.wrapping_add(1);
            self.window_pixel_count = 0;
        }

        // Determine tile source
        let (map_base, col, row, tile_x, tile_y) = if self.window_active {
            let win_base = if self.lcdc & 0x40 != 0 { 0x9C00u16 } else { 0x9800u16 };
            let win_row = self.window_line.wrapping_sub(1);
            (win_base,
             (self.window_pixel_count / 8) as u16,
             (win_row / 8) as u16,
             self.window_pixel_count % 8,
             win_row & 7)
        } else {
            let sx = self.scx.wrapping_add(self.pixel_x);
            let sy = self.scy.wrapping_add(ly);
            (if self.lcdc & 0x08 != 0 { 0x9C00u16 } else { 0x9800u16 },
             (sx / 8) as u16, (sy / 8) as u16, sx % 8, sy % 8)
        };

        // Read tile from map
        let map_addr = map_base + row * 32 + col;
        let map_idx = (map_addr & 0x1FFF) as usize;
        let tile = vram[map_idx];

        let attr = if cgb_game { vram[0x2000 + map_idx] } else { 0 };
        let bank = if cgb_game { ((attr >> 3) & 0x01) as usize } else { 0 };
        let hflip = cgb_game && (attr & 0x20) != 0;
        let vflip = cgb_game && (attr & 0x40) != 0;
        let eff_y = if vflip { 7 - tile_y } else { tile_y };

        let signed = self.lcdc & 0x10 == 0;
        let tile_addr = if signed {
            (0x9000u16).wrapping_add_signed((tile as i8 as i16).wrapping_mul(16))
        } else {
            0x8000u16 + tile as u16 * 16
        };
        let row_addr = tile_addr + eff_y as u16 * 2;
        let ri = (row_addr & 0x1FFF) as usize;
        let low = vram[bank * 0x2000 + ri];
        let high = vram[bank * 0x2000 + ri + 1];
        let bit = if hflip { tile_x } else { 7 - tile_x };
        let color = ((low >> bit) & 1) | (((high >> bit) & 1) << 1);

        let pixel = if cgb_game {
            Self::cgb_color(bg_palette[((attr & 0x07) as usize) * 4 + color as usize])
        } else if cgb_mode {
            Self::cgb_color(bg_palette[((self.bgp >> (color * 2)) & 0x03) as usize])
        } else {
            Self::shade_to_pixel((self.bgp >> (color * 2)) & 0x03)
        };

        if self.window_active { self.window_pixel_count = self.window_pixel_count.wrapping_add(1); }
        self.pixel_x += 1;
        if self.pixel_x >= 160 { self.complete = true; }
        Some(pixel)
    }

    // ── Public accessors ──

    pub(super) fn pixel_x(&self) -> u8 { self.pixel_x }
    pub(super) fn fine_scroll_x(&self) -> u8 { self.fine_scroll }
    pub(super) fn complete(&self) -> bool { self.complete }

    // ── Register write queue ──

    fn fetch_pixel_x(&self) -> u8 {
        ((self.dot / 8) * 8).min(159) as u8
    }

    pub(super) fn queue_register_write(&mut self, register: u16, value: u8, old_value: u8) -> u8 {
        let changed = old_value ^ value;
        let apply_x = match register {
            0xFF42 | 0xFF43 => self.fetch_pixel_x(),
            0xFF40 if changed & 0x08 != 0 || changed & 0x10 != 0 || changed & 0x40 != 0 => self.fetch_pixel_x(),
            0xFF4A | 0xFF4B => self.pixel_x.saturating_add(6).min(159),
            _ => self.pixel_x,
        }.min(159);
        self.pending_writes.push(PendingWrite { pixel_x: apply_x, register, value });
        apply_x
    }

    fn apply_pending_writes(&mut self) {
        self.pending_writes.retain(|w| {
            if w.pixel_x <= self.pixel_x {
                match w.register {
                    0xFF40 => {
                        let old_lcdc = self.lcdc;
                        self.lcdc = w.value;
                        // WIN_EN toggle: deactivate/reactivate immediately
                        if old_lcdc & 0x20 != 0 && w.value & 0x20 == 0 {
                            self.window_active = false;
                        }
                    }
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

    // ── Conversion ──

    fn cgb_color(c: u16) -> u32 {
        let r = ((c >> 0) & 0x1F) as u32;
        let g = ((c >> 5) & 0x1F) as u32;
        let b = ((c >> 10) & 0x1F) as u32;
        ((r << 3 | r >> 2) << 24) | ((g << 3 | g >> 2) << 16) | ((b << 3 | b >> 2) << 8) | 0xFF
    }

    fn shade_to_pixel(shade: u8) -> u32 {
        match shade { 0 => 0xFFFF_FFFF, 1 => 0xAAAA_AAFF, 2 => 0x5555_55FF, _ => 0x0000_00FF }
    }
}
