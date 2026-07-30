use std::collections::VecDeque;

#[derive(Debug, Clone, Copy)]
struct PendingWrite {
    pixel_x: u8,
    register: u16,
    value: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FetchStage {
    Tile,
    DataLow,
    DataHigh,
    Sleep,
    Push,
}

#[derive(Debug, Clone)]
pub(super) struct Mode3Pipeline {
    // ── Fetcher ──
    fetch_stage: FetchStage,
    /// Dot within current fetch stage (0-1, each stage is 2 dots)
    fetch_subdot: u8,
    /// Fetcher pixel position (tile-aligned, advances by 8 each tile)
    fetch_pixel_x: u8,
    /// Platch tile index from map
    fetch_tile: u8,
    /// Platch tile attributes (CGB: palette, bank, flip)
    fetch_attr: u8,
    /// Tile bitplane data: [0]=plow, [1]=phigh (each 8 bytes)
    fetch_row: [u8; 2],
    /// Row within the tile (0-7)
    tile_y: u8,

    // ── BG FIFO ──
    bg_fifo: VecDeque<u8>,

    // ── Output ──
    pixel_x: u8,
    complete: bool,

    // ── Stalls ──
    /// Startup delay (SCX fine scroll + initial pipeline fill)
    startup_remaining: u8,
    /// General stall counter (sprite fetch, window restart)
    stall: u8,
    /// Fine scroll: number of initial pixels to discard
    fine_scroll: u8,
    /// bg_fifo discard counter for fine scroll
    discard: u8,

    // ── Window state (simplified: per-pixel active flag) ──
    /// Whether the PPU is currently rendering window content
    pub(super) window_active: bool,
    pub(super) window_line: u8,

    // ── Latched registers (updated by register writes during mode 3) ──
    pub(super) wx: u8,
    pub(super) wy: u8,
    pub(super) lcdc: u8,
    scx: u8,
    scy: u8,
    pub(super) bgp: u8,
    pub(super) obp0: u8,
    pub(super) obp1: u8,

    // ── Sprites ──
    sprite_x: Vec<i16>,
    next_sprite: usize,
    last_sprite_tile: Option<i16>,

    // ── Pending register writes (applied at pixel_x) ──
    pending_writes: Vec<PendingWrite>,
}

impl Mode3Pipeline {
    pub(super) fn new(
        cgb_mode: bool, scx: u8, scy: u8, ly: u8,
        sprite_x: Vec<i16>,
        wx: u8, wy: u8, lcdc: u8,
    ) -> Self {
        Self {
            fetch_stage: FetchStage::Tile,
            fetch_subdot: 0,
            fetch_pixel_x: 0,
            fetch_tile: 0,
            fetch_attr: 0,
            fetch_row: [0, 0],
            tile_y: ly.wrapping_add(scy) & 7,
            bg_fifo: VecDeque::with_capacity(16),
            pixel_x: 0,
            complete: false,
            startup_remaining: if cgb_mode { 19 } else { 18 } + (scx & 7),
            stall: 0,
            fine_scroll: scx & 7,
            discard: 0,
            window_active: false,
            window_line: 0,
            wx, wy, lcdc,
            scx, scy,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            sprite_x,
            next_sprite: 0,
            last_sprite_tile: None,
            pending_writes: Vec::new(),
        }
    }

    /// Advance the pipeline by one dot. Returns a pixel when one is ready.
    pub(super) fn step(
        &mut self,
        vram: &[u8; 0x4000],
        bg_palette: &[u16; 32],
        cgb_mode: bool,
        cgb_game: bool,
        ly: u8,
    ) -> Option<u32> {
        if self.complete {
            return None;
        }

        // Apply pending register writes whose pixel_x has been reached
        self.apply_pending_writes();

        // 1. Startup phase: initial pipeline fill + SCX fine scroll
        if self.startup_remaining > 0 {
            self.startup_remaining -= 1;
            self.advance_fetcher(vram, cgb_mode, ly);
            if self.startup_remaining == 0 && self.wx == 0 && self.lcdc & 0x20 != 0 && ly >= self.wy {
                self.window_active = true;
                self.window_line = self.window_line.wrapping_add(1);
                self.bg_fifo.clear();
                self.stall = 6;
            }
            return None;
        }

        // 2. Handle general stalls
        if self.stall > 0 {
            self.stall -= 1;
            return None;
        }

        // 3. Window startup check (transition from BG to Window)
        let window_x = self.wx as i16 - 7;
        if !self.window_active
            && self.lcdc & 0x20 != 0
            && ly >= self.wy
            && window_x < 160
            && self.pixel_x as i16 >= window_x.max(0)
        {
            self.window_active = true;
            self.window_line = self.window_line.wrapping_add(1);
            self.bg_fifo.clear();
            self.stall = 6;
            self.fetch_subdot = 0;
            self.fetch_stage = FetchStage::Tile;
            return None;
        }

        // 4. Fill FIFO from fetcher
        self.fill_fifo(vram, cgb_mode, ly);

        // 5. Check sprite scheduling
        self.check_sprites();

        if self.stall > 0 {
            self.stall -= 1;
            return None;
        }

        // 6. Pop pixel from FIFO
        if let Some(bg_color) = self.bg_fifo.pop_front() {
            // Fine scroll discard: skip pixels without advancing output
            if self.discard < self.fine_scroll {
                self.discard += 1;
                return None;
            }

            // Convert color to RGBA
            let pixel = if self.lcdc & 0x01 == 0 && !self.window_active {
                0xFF_FF_FF_FF // BG disabled
            } else {
                if cgb_game {
                    // CGB native: palette from attribute byte
                    let attr = self.fetch_attr;
                    let pal = (attr & 0x07) as usize;
                    Self::cgb_color(bg_palette[pal * 4 + bg_color as usize])
                } else if cgb_mode {
                    // DMG game on CGB: DMG palette index → CGB palette 0
                    let shade = (self.bgp >> (bg_color * 2)) & 0x03;
                    Self::cgb_color(bg_palette[shade as usize])
                } else {
                    // Pure DMG
                    let shade = (self.bgp >> (bg_color * 2)) & 0x03;
                    Self::shade_to_pixel(shade)
                }
            };

            self.pixel_x += 1;
            if self.pixel_x >= 160 {
                self.complete = true;
            }
            Some(pixel)
        } else {
            // FIFO empty: pixel not ready yet
            None
        }
    }

    // ── Fetcher ──

    fn advance_fetcher(&mut self, vram: &[u8; 0x4000], cgb_mode: bool, ly: u8) {
        // 2 dots per fetch stage
        self.fetch_subdot += 1;
        if self.fetch_subdot < 2 {
            return;
        }
        self.fetch_subdot = 0;

        self.fetch_stage = match self.fetch_stage {
            FetchStage::Tile => {
                // Read tile index from map
                let _signed = self.lcdc & 0x10 == 0;
                let map_base: u16 = if self.window_active {
                    if self.lcdc & 0x40 != 0 { 0x9C00 } else { 0x9800 }
                } else {
                    if self.lcdc & 0x08 != 0 { 0x9C00 } else { 0x9800 }
                };
                
                let tile_col: u16;
                let tile_row: u16;
                if self.window_active {
                    tile_col = (self.fetch_pixel_x / 8) as u16;
                    tile_row = self.window_line.wrapping_sub(1) as u16;
                } else {
                    tile_col = ((self.scx as u32 + self.fetch_pixel_x as u32) / 8) as u16 & 0x1F;
                    tile_row = (ly as u32 + self.scy as u32) as u16 & 0xFF;
                };
                let map_addr = map_base + (tile_row / 8) * 32 + tile_col;
                let map_idx = (map_addr & 0x1FFF) as usize;
                self.fetch_tile = vram[map_idx];
                
                if cgb_mode {
                    self.fetch_attr = vram[0x2000 + map_idx];
                } else {
                    self.fetch_attr = 0;
                }

                if self.window_active {
                    self.tile_y = (self.window_line.wrapping_sub(1) as u16 % 8) as u8;
                } else {
                    self.tile_y = (ly.wrapping_add(self.scy) & 7) as u8;
                };
                if cgb_mode && self.fetch_attr & 0x40 != 0 {
                    self.tile_y = 7 - self.tile_y;
                }

                self.fetch_stage = FetchStage::DataLow;
                FetchStage::DataLow
            }
            FetchStage::DataLow => {
                let signed = self.lcdc & 0x10 == 0;
                let tile_addr = if signed {
                    let signed_idx = self.fetch_tile as i8 as i16;
                    (0x9000u16).wrapping_add_signed(signed_idx.wrapping_mul(16))
                } else {
                    0x8000u16 + self.fetch_tile as u16 * 16
                };
                let bank = if cgb_mode { ((self.fetch_attr >> 3) & 0x01) as usize } else { 0 };
                let row_addr = tile_addr + self.tile_y as u16 * 2;
                let row_idx = (row_addr & 0x1FFF) as usize;
                self.fetch_row[0] = vram[bank * 0x2000 + row_idx];
                self.fetch_stage = FetchStage::DataHigh;
                FetchStage::DataHigh
            }
            FetchStage::DataHigh => {
                let signed = self.lcdc & 0x10 == 0;
                let tile_addr = if signed {
                    let signed_idx = self.fetch_tile as i8 as i16;
                    (0x9000u16).wrapping_add_signed(signed_idx.wrapping_mul(16))
                } else {
                    0x8000u16 + self.fetch_tile as u16 * 16
                };
                let bank = if cgb_mode { ((self.fetch_attr >> 3) & 0x01) as usize } else { 0 };
                let row_addr = tile_addr + self.tile_y as u16 * 2;
                let row_idx = (row_addr & 0x1FFF) as usize;
                self.fetch_row[1] = vram[bank * 0x2000 + row_idx + 1];
                self.fetch_stage = FetchStage::Sleep;
                FetchStage::Sleep
            }
            FetchStage::Sleep => {
                FetchStage::Push
            }
            FetchStage::Push => {
                // Push 8 pixels to FIFO
                let hflip = cgb_mode && self.fetch_attr & 0x20 != 0;
                let low = self.fetch_row[0];
                let high = self.fetch_row[1];
                for bit in 0..8 {
                    let b = if hflip { bit } else { 7 - bit };
                    let color = ((low >> b) & 1) | (((high >> b) & 1) << 1);
                    self.bg_fifo.push_back(color);
                }
                self.fetch_pixel_x = self.fetch_pixel_x.wrapping_add(8);
                // Check window restart on each tile push
                if self.lcdc & 0x80 == 0 {
                    // Safety check
                }
                self.fetch_stage = FetchStage::Tile;
                FetchStage::Tile
            }
        };
    }

    fn fill_fifo(&mut self, vram: &[u8; 0x4000], cgb_mode: bool, ly: u8) {
        // Keep FIFO filled: advance fetcher while FIFO < 8
        while self.bg_fifo.len() < 8 {
            self.advance_fetcher(vram, cgb_mode, ly);
        }
    }

    // ── Sprites ──

    fn check_sprites(&mut self) {
        while self.next_sprite < self.sprite_x.len()
            && self.sprite_x[self.next_sprite] <= self.pixel_x as i16
        {
            let sprite_x = self.sprite_x[self.next_sprite];
            let tile = (self.pixel_x as i16 + self.scx as i16) / 8;
            let fetch_wait = if sprite_x == -8 {
                5
            } else if self.last_sprite_tile == Some(tile) {
                0
            } else {
                let tile_x = (self.pixel_x as i16 + self.scx as i16) & 7;
                (5 - tile_x).max(0) as u8
            };
            self.last_sprite_tile = Some(tile);
            self.stall = self.stall.saturating_add(6 + fetch_wait);
            self.next_sprite += 1;
        }
    }

    // ── Conversion helpers ──

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
            0 => 0xFF_FF_FF_FF,
            1 => 0xAA_AA_AA_FF,
            2 => 0x55_55_55_FF,
            _ => 0x00_00_00_FF,
        }
    }

    pub(super) fn pixel_x(&self) -> u8 {
        self.pixel_x
    }

    pub(super) fn fine_scroll_x(&self) -> u8 {
        self.fine_scroll
    }

    pub(super) fn complete(&self) -> bool {
        self.complete
    }

    /// Queue a register write that should take effect at the current pixel_x + 6 (fetcher restart delay).
    pub(super) fn queue_register_write(&mut self, register: u16, value: u8) {
        let apply_x = self.pixel_x.saturating_add(6).min(159);
        self.pending_writes.push(PendingWrite { pixel_x: apply_x, register, value });
    }

    /// Apply any pending writes whose pixel_x has been reached.
    fn apply_pending_writes(&mut self) {
        let before = self.pending_writes.len();
        self.pending_writes.retain(|w| {
            if w.pixel_x <= self.pixel_x {
                match w.register {
                    0xFF47 => self.bgp = w.value,
                    0xFF48 => self.obp0 = w.value,
                    0xFF49 => self.obp1 = w.value,
                    0xFF4A => self.wy = w.value,
                    0xFF4B => self.wx = w.value,
                    0xFF40 => self.lcdc = w.value,
                    0xFF42 => self.scy = w.value,
                    0xFF43 => self.scx = w.value,
                    _ => {}
                }
                false
            } else {
                true
            }
        });
    }
}
