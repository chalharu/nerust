use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchStage {
    Tile,     // Read tile index from map
    DataLow,  // Read bitplane 0
    DataHigh, // Read bitplane 1
    Sleep,
    Push,     // Combine planes → push 8 pixels to FIFO
}

#[derive(Debug, Clone)]
pub(super) struct Mode3Pipeline {
    // ── Fetcher state machine ──
    fetch_stage: FetchStage,
    fetch_dot: u8,          // 0-1 within each stage (each stage = 2 dots)
    fetch_pixel_x: u8,      // tile-aligned X position (0, 8, 16, ...)
    tile_index: u8,         // latched tile index from map
    tile_attr: u8,          // latched attribute byte (CGB)
    tile_y: u8,             // latched row within tile (0-7)
    tile_row_low: u8,       // latched bitplane 0 byte
    tile_row_high: u8,      // latched bitplane 1 byte

    // ── BG pixel FIFO ──
    bg_fifo: VecDeque<u8>,

    // ── Output ──
    pixel_x: u8,
    fine_scroll: u8,        // SCX & 7 — discard from first tile
    discard: u8,            // fine scroll discard counter
    complete: bool,

    // ── Stalls ──
    stall: u8,
    startup: u8,            // initial pipeline fill dots

    // ── Latched registers (updated by pending writes) ──
    pub(super) lcdc: u8,
    scx: u8,
    scy: u8,
    pub(super) wx: u8,
    pub(super) wy: u8,
    pub(super) bgp: u8,

    // ── Window state ──
    pub(super) window_active: bool,
    pub(super) window_line: u8,

    // ── Pending register writes ──
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
        cgb_mode: bool, scx: u8, scy: u8,
        wx: u8, wy: u8, lcdc: u8,
    ) -> Self {
        Self {
            fetch_stage: FetchStage::Tile,
            fetch_dot: 0,
            fetch_pixel_x: 0,
            tile_index: 0,
            tile_attr: 0,
            tile_y: 0,
            tile_row_low: 0,
            tile_row_high: 0,
            bg_fifo: VecDeque::with_capacity(16),
            pixel_x: 0,
            fine_scroll: scx & 7,
            discard: 0,
            complete: false,
            stall: 0,
            startup: if cgb_mode { 19 } else { 18 } + (scx & 7),
            lcdc, scx, scy, wx, wy,
            bgp: 0xFC,
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

        if self.startup > 0 {
            self.startup -= 1;
            self.advance_fetcher(vram, cgb_mode, ly);
            return None;
        }

        if self.stall > 0 {
            self.stall -= 1;
            return None;
        }

        // Keep FIFO filled: advance fetcher while FIFO < 8
        while self.bg_fifo.len() < 8 {
            self.advance_fetcher(vram, cgb_mode, ly);
        }

        // Fine scroll: discard first `fine_scroll` pixels from FIFO once
        while self.discard < self.fine_scroll {
            self.bg_fifo.pop_front();
            self.discard += 1;
        }

        // Pop pixel from FIFO
        if let Some(bg_color) = self.bg_fifo.pop_front() {

            let pixel = if cgb_game {
                let pal = (self.tile_attr & 0x07) as usize;
                Self::cgb_color(bg_palette[pal * 4 + bg_color as usize])
            } else if cgb_mode {
                let shade = (self.bgp >> (bg_color * 2)) & 0x03;
                Self::cgb_color(bg_palette[shade as usize])
            } else {
                let shade = (self.bgp >> (bg_color * 2)) & 0x03;
                Self::shade_to_pixel(shade)
            };

            self.pixel_x += 1;
            if self.pixel_x >= 160 { self.complete = true; }
            return Some(pixel);
        }

        None
    }

    // ── Fetcher: advances one dot through stage pipeline ──

    fn advance_fetcher(&mut self, vram: &[u8; 0x4000], cgb_mode: bool, ly: u8) {
        self.fetch_dot += 1;
        if self.fetch_dot < 2 { return; }
        self.fetch_dot = 0;

        match self.fetch_stage {
            FetchStage::Tile => {
                // Read tile index from background map
                let map_base: u16 = if self.lcdc & 0x08 != 0 { 0x9C00 } else { 0x9800 };
                let tile_col = (self.scx as u32 + self.fetch_pixel_x as u32) as u16 / 8 & 0x1F;
                let tile_row = (ly as u32 + self.scy as u32) as u16 / 8 & 0x1F;
                let map_addr = map_base + tile_row * 32 + tile_col;
                let map_idx = (map_addr & 0x1FFF) as usize;
                self.tile_index = vram[map_idx];
                if cgb_mode {
                    self.tile_attr = vram[0x2000 + map_idx];
                }
                let vflip = cgb_mode && self.tile_attr & 0x40 != 0;
                let raw_y = ly.wrapping_add(self.scy) & 7;
                self.tile_y = if vflip { 7 - raw_y } else { raw_y };
                self.fetch_stage = FetchStage::DataLow;
            }
            FetchStage::DataLow => {
                let signed = self.lcdc & 0x10 == 0;
                let tile_addr = if signed {
                    (0x9000u16).wrapping_add_signed((self.tile_index as i8 as i16).wrapping_mul(16))
                } else {
                    0x8000u16 + self.tile_index as u16 * 16
                };
                let bank = if cgb_mode { ((self.tile_attr >> 3) & 0x01) as usize } else { 0 };
                let row_addr = tile_addr + self.tile_y as u16 * 2;
                let row_idx = (row_addr & 0x1FFF) as usize;
                self.tile_row_low = vram[bank * 0x2000 + row_idx];
                self.fetch_stage = FetchStage::DataHigh;
            }
            FetchStage::DataHigh => {
                let signed = self.lcdc & 0x10 == 0;
                let tile_addr = if signed {
                    (0x9000u16).wrapping_add_signed((self.tile_index as i8 as i16).wrapping_mul(16))
                } else {
                    0x8000u16 + self.tile_index as u16 * 16
                };
                let bank = if cgb_mode { ((self.tile_attr >> 3) & 0x01) as usize } else { 0 };
                let row_addr = tile_addr + self.tile_y as u16 * 2;
                let row_idx = (row_addr & 0x1FFF) as usize;
                self.tile_row_high = vram[bank * 0x2000 + row_idx + 1];
                self.fetch_stage = FetchStage::Sleep;
            }
            FetchStage::Sleep => {
                self.fetch_stage = FetchStage::Push;
            }
            FetchStage::Push => {
                let hflip = cgb_mode && self.tile_attr & 0x20 != 0;
                let low = self.tile_row_low;
                let high = self.tile_row_high;
                for bit in 0..8u8 {
                    let b = if hflip { bit } else { 7 - bit };
                    let color = ((low >> b) & 1) | (((high >> b) & 1) << 1);
                    self.bg_fifo.push_back(color);
                }
                self.fetch_pixel_x = self.fetch_pixel_x.wrapping_add(8);
                self.fetch_stage = FetchStage::Tile;
            }
        }
    }

    // ── Pending write handling ──

    pub(super) fn queue_register_write(&mut self, register: u16, value: u8) {
        let apply_x = self.pixel_x.saturating_add(6).min(159);
        self.pending_writes.push(PendingWrite { pixel_x: apply_x, register, value });
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
            } else {
                true
            }
        });
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

    pub(super) fn pixel_x(&self) -> u8 { self.pixel_x }
    pub(super) fn fine_scroll_x(&self) -> u8 { self.fine_scroll }
    pub(super) fn complete(&self) -> bool { self.complete }
}
