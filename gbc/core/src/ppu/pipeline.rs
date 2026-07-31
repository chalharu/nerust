use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchStage {
    Tile,
    DataLow,
    DataHigh,
    Sleep,
    Push,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Sprite {
    pub(super) x: i16,
    pub(super) tile: u8,
    pub(super) y: i16,
    pub(super) flags: u8,
    pub(super) oam_index: u8,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Registers {
    pub(super) lcdc: u8,
    pub(super) scy: u8,
    pub(super) scx: u8,
    pub(super) wy: u8,
    pub(super) wx: u8,
    pub(super) bgp: u8,
    pub(super) obp0: u8,
    pub(super) obp1: u8,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct OutputPixel {
    pub(super) x: u8,
    pub(super) color: u32,
}

#[derive(Debug, Clone, Copy, Default)]
struct BgPixel {
    color: u8,
    palette: u8,
    priority: bool,
}

#[derive(Debug, Clone, Copy)]
struct ObjPixel {
    color: u8,
    palette: u8,
    behind_bg: bool,
    priority_key: u16,
}

#[derive(Debug)]
struct Fetcher {
    stage: FetchStage,
    stage_dot: u8,
    tile_column: u8,
    tile_index: u8,
    attributes: u8,
    tile_y: u8,
    map_address: u16,
    data_address: usize,
    low: u8,
    high: u8,
    skip_sleep: bool,
}

impl Fetcher {
    fn new(tile_column: u8) -> Self {
        Self {
            stage: FetchStage::Tile,
            stage_dot: 0,
            tile_column,
            tile_index: 0,
            attributes: 0,
            tile_y: 0,
            map_address: 0,
            data_address: 0,
            low: 0,
            high: 0,
            skip_sleep: true,
        }
    }

    fn restart(&mut self, tile_column: u8) {
        *self = Self::new(tile_column);
    }

    fn advance(&mut self) {
        if self.stage == FetchStage::Sleep {
            self.stage_dot = 0;
            self.stage = FetchStage::Push;
            return;
        }
        if self.stage == FetchStage::Push {
            self.stage_dot = 0;
            self.tile_column = self.tile_column.wrapping_add(1);
            self.stage = FetchStage::Tile;
            return;
        }
        self.stage_dot += 1;
        if self.stage_dot != 2 {
            return;
        }
        self.stage_dot = 0;
        self.stage = match self.stage {
            FetchStage::Tile => FetchStage::DataLow,
            FetchStage::DataLow => FetchStage::DataHigh,
            FetchStage::DataHigh if self.skip_sleep => {
                self.skip_sleep = false;
                FetchStage::Push
            }
            FetchStage::DataHigh => FetchStage::Sleep,
            FetchStage::Sleep => unreachable!("sleep advances in one dot"),
            FetchStage::Push => unreachable!("push advances in one dot"),
        };
    }
}

#[derive(Debug)]
struct SpriteFetch {
    sprite: Sprite,
    bg_wait: u8,
    advance_bg: bool,
    dot: u8,
    low: u8,
    high: u8,
    height: u8,
    data_address: usize,
}

#[derive(Debug)]
pub(super) struct Mode3Pipeline {
    registers: Registers,
    written_lcdc: u8,
    ly: u8,
    cgb_mode: bool,
    cgb_game: bool,
    oam_priority: bool,
    startup_dots: u8,
    initial_dummy_pending: bool,
    fine_discard: u8,
    scx_tile_latch: u8,
    pixel_x: u8,
    complete: bool,

    fetcher: Fetcher,
    bg_fifo: VecDeque<BgPixel>,
    window_active: bool,
    window_eligible: bool,
    window_seen: bool,
    window_triggered: bool,
    window_can_retrigger: bool,
    window_zero_at: Option<u8>,
    window_disable_countdown: Option<u8>,
    window_start_delay: u8,
    window_line: u8,
    window_pixels: u8,

    sprites: Vec<Sprite>,
    next_sprite: usize,
    sprite_fetch: Option<SpriteFetch>,
    output_stall: u8,
    last_sprite_tile: Option<i16>,
    pending_bg_enable: Option<(u8, u8)>,
    pending_obj_enable: Option<(u8, u8)>,
    pending_bgp: Option<(u8, u8)>,
    pending_obj_size: Option<u8>,
    pending_scy: Option<(u8, u8)>,
    pending_map_select: Option<(u8, u8)>,
    pending_tile_select: Option<(u8, u8)>,
    refetch_push_map: bool,
    wx_written: bool,
    pending_tile_select_write: Option<(u8, u8)>,
    active_tile_select_write: Option<(u8, u8)>,
    tile_data_bus: u8,
    obj_line: [Option<ObjPixel>; 160],
}

impl Mode3Pipeline {
    pub(super) fn new(
        registers: Registers,
        ly: u8,
        window_line: u8,
        window_eligible: bool,
        mut sprites: Vec<Sprite>,
        cgb_mode: bool,
        cgb_game: bool,
        opri: u8,
    ) -> Self {
        let oam_priority = cgb_game && opri & 1 != 0;
        sprites.sort_by(|a, b| a.x.cmp(&b.x).then_with(|| a.oam_index.cmp(&b.oam_index)));
        Self {
            registers,
            written_lcdc: registers.lcdc,
            ly,
            cgb_mode,
            cgb_game,
            oam_priority,
            startup_dots: if cgb_mode { 19 } else { 18 } + (registers.scx & 7),
            initial_dummy_pending: true,
            fine_discard: registers.scx & 7,
            scx_tile_latch: registers.scx >> 3,
            pixel_x: 0,
            complete: false,
            fetcher: Fetcher::new(registers.scx >> 3),
            bg_fifo: VecDeque::with_capacity(16),
            window_active: false,
            window_eligible,
            window_seen: false,
            window_triggered: false,
            window_can_retrigger: false,
            window_zero_at: None,
            window_disable_countdown: None,
            window_start_delay: 0,
            window_line,
            window_pixels: 0,
            sprites,
            next_sprite: 0,
            sprite_fetch: None,
            output_stall: 0,
            last_sprite_tile: None,
            pending_bg_enable: None,
            pending_obj_enable: None,
            pending_bgp: None,
            pending_obj_size: None,
            pending_scy: None,
            pending_map_select: None,
            pending_tile_select: None,
            refetch_push_map: false,
            wx_written: false,
            pending_tile_select_write: None,
            active_tile_select_write: None,
            tile_data_bus: 0,
            obj_line: [None; 160],
        }
    }

    pub(super) fn step(
        &mut self,
        vram: &[u8; 0x4000],
        bg_palette: &[u16; 32],
        obj_palette: &[u16; 32],
    ) -> Option<OutputPixel> {
        if self.complete {
            return None;
        }
        if let Some(write) = self.pending_tile_select_write.take() {
            self.active_tile_select_write = Some(write);
        }
        if let Some((countdown, value)) = self.pending_scy.as_mut() {
            *countdown = countdown.saturating_sub(1);
            if *countdown == 0 {
                self.registers.scy = *value;
                self.pending_scy = None;
            }
        }
        if let Some((countdown, value)) = self.pending_map_select.as_mut() {
            *countdown = countdown.saturating_sub(1);
            if *countdown == 0 {
                self.registers.lcdc = (self.registers.lcdc & !0x48) | *value;
                self.pending_map_select = None;
            }
        }
        if let Some((countdown, value)) = self.pending_tile_select.as_mut() {
            *countdown = countdown.saturating_sub(1);
            if *countdown == 0 {
                self.registers.lcdc = (self.registers.lcdc & !0x10) | *value;
                self.pending_tile_select = None;
            }
        }
        if self.startup_dots != 0 {
            if self.window_eligible
                && self.registers.lcdc & 0x20 != 0
                && self.ly >= self.registers.wy
                && self.registers.wx <= 7
            {
                self.window_seen = true;
            }
            self.step_bg_fetcher(vram);
            if self.initial_dummy_pending && !self.bg_fifo.is_empty() {
                self.bg_fifo.clear();
                self.fetcher.tile_column = self.fetcher.tile_column.wrapping_sub(1);
                self.initial_dummy_pending = false;
            }
            self.startup_dots -= 1;
            if self.startup_dots == 0 {
                for _ in 0..self.fine_discard {
                    self.bg_fifo.pop_front();
                }
            }
            return None;
        }

        if self.window_start_delay != 0 {
            self.window_start_delay -= 1;
            return None;
        }
        self.start_sprite_fetch();
        if self.sprite_fetch.is_some() {
            self.step_sprite_fetch(vram);
        } else if self.output_stall == 0 {
            self.step_bg_fetcher(vram);
        }
        if let Some(value) = self.pending_obj_size.take() {
            self.registers.lcdc = (self.registers.lcdc & !4) | value;
        }
        if self.output_stall != 0 {
            self.output_stall -= 1;
            return None;
        }
        self.update_window_state();
        let mut bg = self.bg_fifo.pop_front()?;
        let x = self.pixel_x;
        if self.window_zero_at == Some(x) {
            bg = BgPixel::default();
            self.window_zero_at = None;
        }
        let color = self.compose_pixel(bg, self.obj_line[x as usize], bg_palette, obj_palette);
        self.pixel_x += 1;
        if let Some((countdown, value)) = self.pending_bg_enable.as_mut() {
            *countdown = countdown.saturating_sub(1);
            if *countdown == 0 {
                self.registers.lcdc = (self.registers.lcdc & !1) | *value;
                self.pending_bg_enable = None;
            }
        }
        if let Some((countdown, value)) = self.pending_obj_enable.as_mut() {
            *countdown = countdown.saturating_sub(1);
            if *countdown == 0 {
                self.registers.lcdc = (self.registers.lcdc & !2) | *value;
                self.pending_obj_enable = None;
            }
        }
        if let Some((countdown, value)) = self.pending_bgp.as_mut() {
            *countdown = countdown.saturating_sub(1);
            if *countdown == 0 {
                self.registers.bgp = *value;
                self.pending_bgp = None;
            }
        }
        if self.window_active {
            self.window_pixels = self.window_pixels.wrapping_add(1);
            if let Some(countdown) = self.window_disable_countdown.as_mut() {
                *countdown = countdown.saturating_sub(1);
            }
        }
        self.complete = self.pixel_x == 160;
        Some(OutputPixel { x, color })
    }

    fn step_bg_fetcher(&mut self, vram: &[u8; 0x4000]) {
        if self.fetcher.stage == FetchStage::Push && !self.bg_fifo.is_empty() {
            return;
        }
        match self.fetcher.stage {
            FetchStage::Tile if self.fetcher.stage_dot == 0 => self.prepare_tile_address(),
            FetchStage::Tile => self.read_tile(vram),
            FetchStage::DataLow if self.fetcher.stage_dot == 0 => {
                self.prepare_tile_data_address(false)
            }
            FetchStage::DataLow => {
                self.fetcher.low = if self.active_tile_select_write
                    .is_some_and(|(old, new)| old == 0 && new != 0)
                {
                    self.tile_data_bus
                } else {
                    vram[self.fetcher.data_address]
                };
                self.active_tile_select_write = None;
            }
            FetchStage::DataHigh if self.fetcher.stage_dot == 0 => {
                self.prepare_tile_data_address(true)
            }
            FetchStage::DataHigh => {
                self.fetcher.high = match self.active_tile_select_write {
                    Some((0, new)) if new != 0 => self.tile_data_bus,
                    Some((old, 0)) if old != 0 => self.fetcher.low,
                    _ => vram[self.fetcher.data_address],
                };
                self.tile_data_bus = self.fetcher.high;
                self.active_tile_select_write = None;
            }
            FetchStage::Sleep => {}
            FetchStage::Push if self.fetcher.stage_dot == 0 => {
                if self.refetch_push_map {
                    self.refetch_push_map(vram);
                }
                self.push_bg_tile();
            }
            FetchStage::Push => {}
        }
        self.fetcher.advance();
    }

    fn prepare_tile_address(&mut self) {
        if !self.window_active && self.fetcher.stage_dot == 0 {
            let new_column = self.registers.scx >> 3;
            self.fetcher.tile_column = self
                .fetcher
                .tile_column
                .wrapping_add(new_column.wrapping_sub(self.scx_tile_latch));
            self.scx_tile_latch = new_column;
        }
        let (map_base, tile_row, tile_y) = if self.window_active {
            let map = if self.registers.lcdc & 0x40 != 0 { 0x9C00 } else { 0x9800 };
            (map, self.window_line >> 3, self.window_line & 7)
        } else {
            let y = self.registers.scy.wrapping_add(self.ly);
            let map = if self.registers.lcdc & 0x08 != 0 { 0x9C00 } else { 0x9800 };
            (map, y >> 3, y & 7)
        };
        self.fetcher.map_address = map_base
            + u16::from(tile_row) * 32
            + u16::from(self.fetcher.tile_column & 31);
        self.fetcher.tile_y = tile_y;
    }

    fn read_tile(&mut self, vram: &[u8; 0x4000]) {
        let map_index = usize::from(self.fetcher.map_address & 0x1FFF);
        self.fetcher.tile_index = vram[map_index];
        self.fetcher.attributes = if self.cgb_game { vram[0x2000 + map_index] } else { 0 };
        self.fetcher.tile_y = if self.cgb_game && self.fetcher.attributes & 0x40 != 0 {
            7 - self.fetcher.tile_y
        } else {
            self.fetcher.tile_y
        };
    }

    fn prepare_tile_data_address(&mut self, high: bool) {
        let tile_address = if self.registers.lcdc & 0x10 == 0 {
            0x9000u16.wrapping_add_signed(
                (self.fetcher.tile_index as i8 as i16).wrapping_mul(16),
            )
        } else {
            0x8000 + u16::from(self.fetcher.tile_index) * 16
        };
        let bank = if self.cgb_game {
            usize::from((self.fetcher.attributes >> 3) & 1)
        } else {
            0
        };
        let address = tile_address
            + u16::from(self.fetcher.tile_y) * 2
            + u16::from(high);
        self.fetcher.data_address = bank * 0x2000 + usize::from(address & 0x1FFF);
    }

    fn push_bg_tile(&mut self) {
        let flipped = self.cgb_game && self.fetcher.attributes & 0x20 != 0;
        let palette = if self.cgb_game { self.fetcher.attributes & 7 } else { 0 };
        let priority = self.cgb_game && self.fetcher.attributes & 0x80 != 0;
        for x in 0..8 {
            let bit = if flipped { x } else { 7 - x };
            let color = ((self.fetcher.low >> bit) & 1)
                | (((self.fetcher.high >> bit) & 1) << 1);
            self.bg_fifo.push_back(BgPixel { color, palette, priority });
        }
    }

    fn refetch_push_map(&mut self, vram: &[u8; 0x4000]) {
        self.refetch_push_map = false;
        let map_base = if self.registers.lcdc & 0x08 != 0 { 0x9C00 } else { 0x9800 };
        self.fetcher.map_address = map_base + (self.fetcher.map_address & 0x03FF);
        self.read_tile(vram);
        self.prepare_tile_data_address(false);
        self.fetcher.low = vram[self.fetcher.data_address];
        self.prepare_tile_data_address(true);
        self.fetcher.high = vram[self.fetcher.data_address];
    }

    fn update_window_state(&mut self) {
        if self.window_active && self.window_disable_countdown == Some(0) {
            self.window_active = false;
            self.window_disable_countdown = None;
            self.bg_fifo.clear();
            self.fetcher.restart(self.registers.scx >> 3);
            return;
        }

        let window_x = i16::from(self.registers.wx) - 7;
        let can_trigger = !self.window_triggered || self.window_can_retrigger;
        let window_enabled = if self.window_triggered {
            self.registers.lcdc & 0x20 != 0
        } else {
            self.window_eligible
        };
        if !self.window_active
            && can_trigger
            && window_enabled
            && self.ly >= self.registers.wy
            && window_x < 160
            && i16::from(self.pixel_x) >= window_x.max(0)
        {
            if self.window_triggered {
                self.window_line = self.window_line.wrapping_add(1);
            }
            self.window_active = true;
            self.window_seen = true;
            self.window_triggered = true;
            self.window_can_retrigger = false;
            self.window_pixels = if window_x < 0 { (-window_x) as u8 } else { 0 };
            self.bg_fifo.clear();
            self.fetcher.restart(self.window_pixels >> 3);
            self.prepare_tile_address();
            self.fetcher.stage_dot = 1;
            self.fine_discard = self.window_pixels & 7;
            self.window_start_delay = if self.registers.wx == 0 && self.registers.scx & 7 != 0 {
                2
            } else {
                0
            };
        }
    }

    fn start_sprite_fetch(&mut self) {
        if self.sprite_fetch.is_some() {
            return;
        }
        if !self.cgb_mode && self.registers.lcdc & 0x02 == 0 {
            while self.next_sprite < self.sprites.len()
                && self.sprites[self.next_sprite].x <= i16::from(self.pixel_x)
            {
                self.next_sprite += 1;
            }
            return;
        }
        if self.next_sprite >= self.sprites.len()
            || self.sprites[self.next_sprite].x > i16::from(self.pixel_x)
        {
            return;
        }
        let sprite = self.sprites[self.next_sprite];
        self.next_sprite += 1;
        let tile = (i16::from(self.pixel_x) + i16::from(self.registers.scx)) / 8;
        let fetch_wait = if self.last_sprite_tile == Some(tile) {
            0
        } else {
            let tile_x = (i16::from(self.pixel_x) + i16::from(self.registers.scx)) & 7;
            (5 - tile_x).max(0) as u8
        };
        self.last_sprite_tile = Some(tile);
        let stall = if sprite.x < 0 {
            if sprite.x <= -5 {
                (3 - sprite.x) as u8
            } else if sprite.x == -4 {
                7
            } else {
                6
            }
        } else {
            6 + fetch_wait
        };
        self.output_stall = stall;
        self.sprite_fetch = Some(SpriteFetch {
            sprite,
            bg_wait: stall - 6,
            advance_bg: true,
            dot: 0,
            low: 0,
            high: 0,
            height: if self.registers.lcdc & 0x04 != 0 { 16 } else { 8 },
            data_address: 0,
        });
    }

    fn step_sprite_fetch(&mut self, vram: &[u8; 0x4000]) {
        if self.sprite_fetch.as_ref().is_some_and(|fetch| fetch.bg_wait != 0) {
            if self.sprite_fetch.as_ref().unwrap().advance_bg {
                self.step_bg_fetcher(vram);
            }
            self.sprite_fetch.as_mut().unwrap().bg_wait -= 1;
            return;
        }
        let Some(fetch) = self.sprite_fetch.as_mut() else { return };
        if fetch.dot == 2 || (fetch.dot == 4 && self.registers.scx != 0) {
            fetch.height = if self.registers.lcdc & 0x04 != 0 { 16 } else { 8 };
        }
        if fetch.dot == 2 || fetch.dot == 4 {
            let mut tile_y = (i16::from(self.ly) - fetch.sprite.y) as u8
                & (fetch.height - 1);
            if fetch.sprite.flags & 0x40 != 0 {
                tile_y ^= fetch.height - 1;
            }
            let tile = if fetch.height == 16 {
                (fetch.sprite.tile & 0xFE) | u8::from(tile_y >= 8)
            } else {
                fetch.sprite.tile
            };
            let bank = if self.cgb_mode {
                usize::from((fetch.sprite.flags >> 3) & 1)
            } else {
                0
            };
            let address = 0x8000u16
                + u16::from(tile) * 16
                + u16::from(tile_y & 7) * 2
                + u16::from(fetch.dot == 4);
            fetch.data_address = bank * 0x2000 + usize::from(address & 0x1FFF);
        }
        if fetch.dot == 3 || fetch.dot == 5 {
            let value = vram[fetch.data_address];
            if fetch.dot == 3 {
                fetch.low = value
            } else {
                fetch.high = value;
                self.tile_data_bus = value;
            }
        }
        fetch.dot += 1;
        if fetch.dot < 6 {
            return;
        }
        let fetch = self.sprite_fetch.take().expect("sprite fetch is active");
        self.merge_sprite(fetch);
    }

    fn merge_sprite(&mut self, fetch: SpriteFetch) {
        for pixel in 0..8i16 {
            let screen_x = fetch.sprite.x + pixel;
            if !(0..160).contains(&screen_x) {
                continue;
            }
            let tile_x = if fetch.sprite.flags & 0x20 != 0 {
                pixel as u8
            } else {
                7 - pixel as u8
            };
            let color = ((fetch.low >> tile_x) & 1) | (((fetch.high >> tile_x) & 1) << 1);
            if color == 0 {
                continue;
            }
            let x_priority = fetch.sprite.x.clamp(-8, 159) + 8;
            let priority_key = if self.oam_priority {
                u16::from(fetch.sprite.oam_index)
            } else {
                (x_priority as u16) << 8 | u16::from(fetch.sprite.oam_index)
            };
            let candidate = ObjPixel {
                color,
                palette: if self.cgb_game {
                    fetch.sprite.flags & 7
                } else {
                    u8::from(fetch.sprite.flags & 0x10 != 0)
                },
                behind_bg: fetch.sprite.flags & 0x80 != 0,
                priority_key,
            };
            let slot = &mut self.obj_line[screen_x as usize];
            if slot.is_none_or(|existing| priority_key < existing.priority_key) {
                *slot = Some(candidate);
            }
        }
    }

    fn compose_pixel(
        &self,
        bg: BgPixel,
        obj: Option<ObjPixel>,
        bg_palette: &[u16; 32],
        obj_palette: &[u16; 32],
    ) -> u32 {
        let bg_enabled = self.cgb_game || self.registers.lcdc & 0x01 != 0;
        let bg_color = if bg_enabled { bg.color } else { 0 };
        let mut pixel = self.bg_pixel(bg_color, bg.palette, bg_palette);
        if self.registers.lcdc & 0x02 == 0 {
            return pixel;
        }
        let Some(obj) = obj else { return pixel };
        let obj_visible = (self.cgb_game && self.registers.lcdc & 0x01 == 0)
            || bg_color == 0
            || (!bg.priority && !obj.behind_bg);
        if obj_visible {
            pixel = self.obj_pixel(obj, obj_palette);
        }
        pixel
    }

    fn bg_pixel(&self, color: u8, palette: u8, palettes: &[u16; 32]) -> u32 {
        if self.cgb_game {
            Self::cgb_color(palettes[usize::from(palette) * 4 + usize::from(color)])
        } else if self.cgb_mode {
            let shade = (self.registers.bgp >> (color * 2)) & 3;
            Self::cgb_color(palettes[usize::from(shade)])
        } else {
            Self::dmg_color((self.registers.bgp >> (color * 2)) & 3)
        }
    }

    fn obj_pixel(&self, pixel: ObjPixel, palettes: &[u16; 32]) -> u32 {
        if self.cgb_game {
            Self::cgb_color(
                palettes[usize::from(pixel.palette) * 4 + usize::from(pixel.color)],
            )
        } else if self.cgb_mode {
            let palette = if pixel.palette == 0 {
                self.registers.obp0
            } else {
                self.registers.obp1
            };
            Self::cgb_color(palettes[usize::from((palette >> (pixel.color * 2)) & 3)])
        } else {
            let palette = if pixel.palette == 0 {
                self.registers.obp0
            } else {
                self.registers.obp1
            };
            Self::dmg_color((palette >> (pixel.color * 2)) & 3)
        }
    }

    fn cgb_color(color: u16) -> u32 {
        let r = u32::from(color & 0x1F);
        let g = u32::from((color >> 5) & 0x1F);
        let b = u32::from((color >> 10) & 0x1F);
        (((r << 3) | (r >> 2)) << 24)
            | (((g << 3) | (g >> 2)) << 16)
            | (((b << 3) | (b >> 2)) << 8)
            | 0xFF
    }

    fn dmg_color(shade: u8) -> u32 {
        match shade {
            0 => 0xFFFF_FFFF,
            1 => 0xAAAA_AAFF,
            2 => 0x5555_55FF,
            _ => 0x0000_00FF,
        }
    }

    pub(super) fn write_register(&mut self, register: u16, value: u8) {
        match register {
            0xFF40 => {
                let old = self.registers.lcdc;
                let old_written = self.written_lcdc;
                self.written_lcdc = value;
                let changed = old_written ^ value;
                let defer_obj_size = changed & 4 != 0
                    && value & 4 == 0
                    && self.registers.scx != 0
                    && self.sprite_fetch.as_ref().is_some_and(|fetch| fetch.dot == 2);
                let preserve = 0x5B | if defer_obj_size { 4 } else { 0 };
                self.registers.lcdc = (value & !preserve) | (old & preserve);
                if changed & 1 != 0 {
                    let delay = if self.fetcher.stage == FetchStage::Tile
                        && self.fetcher.stage_dot == 0
                    {
                        0
                    } else {
                        2u8.saturating_sub(self.output_stall)
                    };
                    if delay == 0 {
                        self.registers.lcdc = (self.registers.lcdc & !1) | (value & 1);
                        self.pending_bg_enable = None;
                    } else {
                        self.pending_bg_enable = Some((delay, value & 1));
                    }
                }
                if changed & 2 != 0 {
                    if self.output_stall >= 2 {
                        self.registers.lcdc = (self.registers.lcdc & !2) | (value & 2);
                        self.pending_obj_enable = None;
                    } else {
                        self.pending_obj_enable = Some((2, value & 2));
                    }
                }
                if defer_obj_size {
                    self.pending_obj_size = Some(value & 4);
                }
                if changed & 0x08 != 0 {
                    if let Some(fetch) = self.sprite_fetch.as_mut() {
                        fetch.advance_bg = false;
                    }
                    let last_object_x = self
                        .next_sprite
                        .checked_sub(1)
                        .map(|i| self.sprites[i].x);
                    let immediate = self.output_stall >= 2
                        && (last_object_x == Some(-8)
                            || self.fetcher.stage == FetchStage::Tile);
                    if immediate {
                        self.registers.lcdc =
                            (self.registers.lcdc & !0x48) | (value & 0x48);
                        self.pending_map_select = None;
                        self.refetch_push_map = last_object_x == Some(-8)
                            && self.fetcher.stage == FetchStage::Push
                            && !self.window_active
                            && changed & 0x08 != 0;
                    } else {
                        self.pending_map_select = Some((4, value & 0x48));
                    }
                }
                if changed & 0x40 != 0 {
                    if let Some(fetch) = self.sprite_fetch.as_mut() {
                        fetch.advance_bg = false;
                    }
                    let object_x = self
                        .next_sprite
                        .checked_sub(1)
                        .map(|i| self.sprites[i].x);
                    let initial_offscreen_set = !self.window_active
                        && value & 0x40 != 0
                        && object_x == Some(-8)
                        && self.output_stall >= 2;
                    let immediate_active = self.window_active
                        && self.output_stall < 2
                        && object_x.is_none_or(|x| x >= 0);
                    if initial_offscreen_set || immediate_active {
                        self.registers.lcdc =
                            (self.registers.lcdc & !0x40) | (value & 0x40);
                        self.pending_map_select = None;
                    } else {
                        let delay = if self.window_active && self.output_stall >= 8 {
                            self.output_stall.saturating_add(5)
                        } else if self.window_active && self.output_stall >= 2 {
                            4u8.max(self.output_stall)
                        } else {
                            4
                        };
                        self.pending_map_select = Some((delay, value & 0x48));
                    }
                }
                if changed & 0x10 != 0 {
                    self.pending_tile_select_write =
                        Some((old_written & 0x10, value & 0x10));
                    self.pending_tile_select = Some((3, value & 0x10));
                }
                if old_written & 0x20 != 0 && value & 0x20 == 0 {
                    if self.window_active {
                        let pixels_left = 8 - (self.window_pixels & 7);
                        self.window_disable_countdown = Some(pixels_left + 8);
                    } else {
                        self.window_triggered = true;
                        self.window_seen = true;
                    }
                }
            }
            0xFF42 => self.pending_scy = Some((4, value)),
            0xFF43 => {
                if self.ly != 0
                    && self.pixel_x == 0
                    && self.fetcher.stage == FetchStage::Tile
                {
                    self.fine_discard = value & 7;
                }
                self.registers.scx = value;
            }
            0xFF47 => {
                if self.registers.wx == 0
                    && self.registers.scx & 7 == 0
                    && self.registers.lcdc & 0x20 != 0
                    && self.window_eligible
                    && !self.wx_written
                    && value != 0
                {
                    self.pending_bgp = Some((7, value));
                } else {
                    self.registers.bgp = value;
                    self.pending_bgp = None;
                }
            }
            0xFF48 => self.registers.obp0 = value,
            0xFF49 => self.registers.obp1 = value,
            0xFF4A => self.registers.wy = value,
            0xFF4B => {
                self.wx_written = true;
                self.window_zero_at = None;
                self.registers.wx = value;
                if self.window_seen {
                    self.window_can_retrigger = true;
                    let window_x = i16::from(value) - 7;
                    if window_x > i16::from(self.pixel_x) && value.saturating_sub(7) & 7 == 5 {
                        self.window_zero_at = Some(value.saturating_sub(7));
                    } else if self.window_triggered && window_x == i16::from(self.pixel_x) {
                        self.window_zero_at = Some(self.pixel_x);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn complete(&self) -> bool {
        self.complete
    }

    pub(super) fn final_window_line(&self) -> u8 {
        self.window_line.wrapping_add(u8::from(self.window_seen))
    }


}

#[cfg(test)]
mod tests {
    use super::*;

    fn registers() -> Registers {
        Registers {
            lcdc: 0x91,
            scy: 0,
            scx: 0,
            wy: 0,
            wx: 0,
            bgp: 0xE4,
            obp0: 0xE4,
            obp1: 0xE4,
        }
    }

    #[test]
    fn fetcher_reads_each_stage_for_two_dots() {
        let mut fetcher = Fetcher::new(0);
        for stage in [
            FetchStage::Tile,
            FetchStage::DataLow,
            FetchStage::DataHigh,
        ] {
            assert_eq!(fetcher.stage, stage);
            fetcher.advance();
            assert_eq!(fetcher.stage, stage);
            fetcher.advance();
        }
        assert_eq!(fetcher.stage, FetchStage::Push);
        fetcher.advance();
        assert_eq!(fetcher.stage, FetchStage::Tile);
        assert_eq!(fetcher.tile_column, 1);
    }

    #[test]
    fn pipeline_outputs_160_pixels() {
        let mut pipeline =
            Mode3Pipeline::new(registers(), 0, 0, false, Vec::new(), false, false, 0);
        let vram = [0; 0x4000];
        let palettes = [0; 32];
        let mut pixels = 0;
        for _ in 0..400 {
            pixels += usize::from(pipeline.step(&vram, &palettes, &palettes).is_some());
            if pipeline.complete() {
                break;
            }
        }
        assert_eq!(pixels, 160);
        assert!(pipeline.complete());
    }
}
