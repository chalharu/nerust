use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchStage {
    Tile,
    DataLow,
    DataHigh,
    Sleep,
    Push,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BgDataRead {
    Low,
    High,
}

#[derive(Debug, Clone, Copy)]
enum WindowReload {
    Both,
    Low,
    CopyLowToHigh,
    CopyNextLowToHigh,
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
    cgb_revision_d: bool,
    oam_priority: bool,
    startup_dots: u8,
    initial_dummy_pending: bool,
    fine_discard: u8,
    scx_tile_latch: u8,
    pending_scx_high: Option<u8>,
    pixel_x: u8,
    complete: bool,

    fetcher: Fetcher,
    bg_fifo: VecDeque<BgPixel>,
    window_active: bool,
    window_eligible: bool,
    window_comparator_seen: bool,
    window_seen: bool,
    window_triggered: bool,
    window_can_retrigger: bool,
    window_activation_pending: bool,
    window_nametable_phase: u8,
    window_trigger_at: Option<u8>,
    window_zero_at: Option<u8>,
    window_disable_countdown: Option<u8>,
    window_start_delay: u8,
    window_line: u8,
    window_pixels: u8,

    sprites: Vec<Sprite>,
    next_sprite: usize,
    sprite_fetch: Option<SpriteFetch>,
    output_stall: u8,
    /// Extra mode-3 dots added by sprite fetches (extend the pixel transfer
    /// period beyond the base 172 dots).
    sprite_extra_dots: u8,
    pending_bg_enable: Option<(u8, u8)>,
    pending_obj_enable: Option<(u8, u8)>,
    pending_bgp: Option<(u8, u8)>,
    pending_obp: Option<(u8, u8, u8)>,
    pending_obj_size: Option<(u8, u8)>,
    relatch_obj_size_high: bool,
    pending_scy: Option<(u8, u8)>,
    scy_written: bool,
    pending_map_select: Option<(u8, u8)>,
    pending_tile_select: Option<(u8, u8)>,
    refetch_push_map: bool,
    wx_written: bool,
    pending_tile_select_write: Option<(u8, u8)>,
    active_tile_select_write: Option<(u8, u8)>,
    cgb_c_tile_write_persistent: bool,
    cgb_c_high_glitch: Option<(u8, u8)>,
    reload_window_tile: Option<WindowReload>,
    last_bg_data_read: Option<BgDataRead>,
    tile_data_bus: u8,
    object_data_bus: Option<u8>,
    sprite_scy_latch: Option<u8>,
    last_output: Option<(u8, u8, [u32; 4])>,
    corrected_output: Option<OutputPixel>,
    force_bg_high_delay: u8,
    force_bg_high_pixels: u8,
    force_bg_low: u8,
    force_bg_low_pixels: u8,
    obj_line: [Option<ObjPixel>; 160],
}

impl Mode3Pipeline {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        registers: Registers,
        ly: u8,
        window_line: u8,
        window_eligible: bool,
        mut sprites: Vec<Sprite>,
        cgb_mode: bool,
        cgb_game: bool,
        cgb_revision_d: bool,
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
            cgb_revision_d,
            oam_priority,
            startup_dots: 19 + (registers.scx & 7),
            initial_dummy_pending: true,
            fine_discard: registers.scx & 7,
            scx_tile_latch: registers.scx >> 3,
            pending_scx_high: None,
            pixel_x: 0,
            complete: false,
            fetcher: Fetcher::new(registers.scx >> 3),
            bg_fifo: VecDeque::with_capacity(16),
            window_active: false,
            window_eligible,
            window_comparator_seen: false,
            window_seen: false,
            window_triggered: false,
            window_can_retrigger: false,
            window_activation_pending: false,
            window_nametable_phase: 5,
            window_trigger_at: None,
            window_zero_at: None,
            window_disable_countdown: None,
            window_start_delay: 0,
            window_line,
            window_pixels: 0,
            sprites,
            next_sprite: 0,
            sprite_fetch: None,
            output_stall: 0,
            sprite_extra_dots: 0,
            pending_bg_enable: None,
            pending_obj_enable: None,
            pending_bgp: None,
            pending_obp: None,
            pending_obj_size: None,
            relatch_obj_size_high: false,
            pending_scy: None,
            scy_written: false,
            pending_map_select: None,
            pending_tile_select: None,
            refetch_push_map: false,
            wx_written: false,
            pending_tile_select_write: None,
            active_tile_select_write: None,
            cgb_c_tile_write_persistent: false,
            cgb_c_high_glitch: None,
            reload_window_tile: None,
            last_bg_data_read: None,
            tile_data_bus: 0,
            object_data_bus: None,
            sprite_scy_latch: None,
            last_output: None,
            corrected_output: None,
            force_bg_high_delay: 0,
            force_bg_high_pixels: 0,
            force_bg_low: 0,
            force_bg_low_pixels: 0,
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
        self.last_bg_data_read = None;
        self.deliver_register_writes();
        if self.window_activation_pending {
            self.window_activation_pending = false;
            self.try_activate_window();
        }
        if self.step_startup(vram) {
            self.advance_dot_writes(!self.cgb_revision_d);
            return None;
        }
        if self.window_start_delay != 0 {
            self.window_start_delay -= 1;
            self.advance_dot_writes(!self.cgb_revision_d);
            return None;
        }
        self.step_fetch_and_sprites(vram);
        if self.output_stall != 0 {
            self.output_stall -= 1;
            self.advance_dot_writes(!self.cgb_revision_d);
            return None;
        }
        let Some(output) = self.emit_output_pixel(bg_palette, obj_palette) else {
            self.advance_dot_writes(!self.cgb_revision_d);
            return None;
        };
        self.advance_pending_countdowns();
        Some(output)
    }

    pub(super) fn set_wx_written_during_oam(&mut self, written: bool) {
        self.wx_written = written;
        if written {
            self.window_nametable_phase = self.registers.wx.wrapping_add(1) & 7;
        }
        self.window_activation_pending = written
            && !self.cgb_mode
            && self.window_eligible
            && (4..=5).contains(&self.registers.wx)
            && self.sprites.is_empty();
    }

    fn deliver_register_writes(&mut self) {
        if self.cgb_revision_d {
            if let Some(write) = self.pending_tile_select_write.take() {
                self.active_tile_select_write = Some(write);
            }
        } else {
            if let Some(write) = self.pending_tile_select_write.take() {
                self.active_tile_select_write = Some(write);
            } else if !self.cgb_c_tile_write_persistent {
                self.active_tile_select_write = None;
            }
        }
        if self.cgb_revision_d {
            self.advance_scy_write();
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
    }

    fn step_startup(&mut self, vram: &[u8; 0x4000]) -> bool {
        if self.startup_dots == 0 {
            return false;
        }
        if self.window_eligible
            && self.registers.lcdc & 0x20 != 0
            && self.ly >= self.registers.wy
            && self.registers.wx <= 7
        {
            self.window_comparator_seen = true;
        }
        self.step_bg_fetcher(vram);
        if !self.cgb_revision_d {
            self.advance_scy_write();
        }
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
        true
    }

    fn step_fetch_and_sprites(&mut self, vram: &[u8; 0x4000]) {
        self.start_sprite_fetch();
        if self.sprite_fetch.is_some() {
            self.step_sprite_fetch(vram);
        } else if self.output_stall == 0 {
            self.step_bg_fetcher(vram);
        }
        if !self.cgb_revision_d {
            self.advance_scy_write();
        }
        self.apply_window_reload(vram);
        if let Some((countdown, value)) = self.pending_obj_size.as_mut() {
            *countdown = countdown.saturating_sub(1);
            if *countdown == 0 {
                self.registers.lcdc = (self.registers.lcdc & !4) | *value;
                self.pending_obj_size = None;
            }
        }
    }

    fn emit_output_pixel(
        &mut self,
        bg_palette: &[u16; 32],
        obj_palette: &[u16; 32],
    ) -> Option<OutputPixel> {
        self.update_window_state();
        let x = self.pixel_x;
        let mut bg = if self.window_zero_at == Some(x) {
            self.window_zero_at = None;
            BgPixel::default()
        } else {
            self.bg_fifo.pop_front()?
        };
        if self.force_bg_high_delay != 0 {
            self.force_bg_high_delay -= 1;
        } else if self.force_bg_high_pixels != 0 {
            bg.color |= 2;
            self.force_bg_high_pixels -= 1;
        }
        if self.force_bg_low_pixels != 0 {
            let bit = self.force_bg_low_pixels - 1;
            bg.color = (bg.color & 2) | ((self.force_bg_low >> bit) & 1);
            self.force_bg_low_pixels -= 1;
        }
        let obj = self.obj_line[x as usize];
        let color = self.compose_pixel(bg, obj, bg_palette, obj_palette);
        let candidates = std::array::from_fn(|candidate| {
            let mut candidate_bg = bg;
            candidate_bg.color = candidate as u8;
            self.compose_pixel(candidate_bg, obj, bg_palette, obj_palette)
        });
        self.last_output = Some((x, bg.color, candidates));
        self.pixel_x += 1;
        if self.window_active {
            self.window_pixels = self.window_pixels.wrapping_add(1);
            if let Some(countdown) = self.window_disable_countdown.as_mut() {
                *countdown = countdown.saturating_sub(1);
            }
        }
        self.complete = self.pixel_x == 160;
        Some(OutputPixel { x, color })
    }

    fn advance_pending_countdowns(&mut self) {
        self.advance_dot_writes(true);
    }

    fn advance_dot_writes(&mut self, advance_lcdc: bool) {
        if advance_lcdc && let Some((countdown, value)) = self.pending_bg_enable.as_mut() {
            *countdown = countdown.saturating_sub(1);
            if *countdown == 0 {
                self.registers.lcdc = (self.registers.lcdc & !1) | *value;
                self.pending_bg_enable = None;
            }
        }
        if advance_lcdc && let Some((countdown, value)) = self.pending_obj_enable.as_mut() {
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
        if let Some((countdown, register, value)) = self.pending_obp.as_mut() {
            *countdown = countdown.saturating_sub(1);
            if *countdown == 0 {
                if *register == 0 {
                    self.registers.obp0 = *value;
                } else {
                    self.registers.obp1 = *value;
                }
                self.pending_obp = None;
            }
        }
    }

    fn step_bg_fetcher(&mut self, vram: &[u8; 0x4000]) {
        if self.step_bg_push_wait(vram) {
            return;
        }
        match self.fetcher.stage {
            FetchStage::Tile if self.fetcher.stage_dot == 0 => self.prepare_tile_address(),
            FetchStage::Tile => self.read_tile(vram),
            FetchStage::DataLow if self.fetcher.stage_dot == 0 => {
                self.prepare_tile_data_address(false)
            }
            FetchStage::DataLow => self.fetch_data_low(vram),
            FetchStage::DataHigh if self.fetcher.stage_dot == 0 => {
                self.prepare_tile_data_address(true)
            }
            FetchStage::DataHigh => self.fetch_data_high(vram),
            FetchStage::Sleep => {}
            FetchStage::Push if self.fetcher.stage_dot == 0 => self.fetch_push(vram),
            FetchStage::Push => {}
        }
        self.fetcher.advance();
    }

    fn step_bg_push_wait(&mut self, vram: &[u8; 0x4000]) -> bool {
        if self.fetcher.stage != FetchStage::Push || self.bg_fifo.is_empty() {
            return false;
        }
        let last_sprite_x = self
            .next_sprite
            .checked_sub(1)
            .map(|index| self.sprites[index].x);
        let reload_low_only = self.cgb_revision_d
            && !self.window_active
            && last_sprite_x == Some(8)
            && self.fetcher.tile_y == 0
            && self
                .active_tile_select_write
                .is_some_and(|(old, new)| old != 0 && new == 0);
        if reload_low_only {
            let lcdc = self.registers.lcdc;
            self.registers.lcdc |= 0x10;
            self.prepare_tile_data_address(false);
            self.fetcher.low = vram[self.fetcher.data_address];
            self.registers.lcdc = lcdc;
            self.active_tile_select_write = None;
        }
        let offscreen_tile_select = if self.cgb_revision_d && !self.window_active {
            match (last_sprite_x, self.active_tile_select_write) {
                (Some(x), Some((old, 0))) if x <= -7 && old != 0 => Some((old, false)),
                (Some(-6), Some((0, new))) if new != 0 => Some((new, false)),
                (Some(x), Some((0, new))) if (-5..=-4).contains(&x) && new != 0 => {
                    Some((new, true))
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some((tile_select, high_only)) = offscreen_tile_select {
            let lcdc = self.registers.lcdc;
            self.registers.lcdc = (lcdc & !0x10) | tile_select;
            if !high_only {
                self.prepare_tile_data_address(false);
                self.fetcher.low = vram[self.fetcher.data_address];
            }
            self.prepare_tile_data_address(true);
            self.fetcher.high = vram[self.fetcher.data_address];
            self.registers.lcdc = lcdc;
            self.active_tile_select_write = None;
        }
        true
    }

    fn fetch_data_low(&mut self, vram: &[u8; 0x4000]) {
        let reset_glitch = !self.cgb_revision_d
            && self
                .active_tile_select_write
                .is_some_and(|(old, new)| old != 0 && new == 0 && self.cgb_c_tile_write_persistent);
        self.fetcher.low = if self.active_tile_select_write.is_some_and(|(old, new)| {
            old == 0 && new != 0 && (self.cgb_revision_d || self.cgb_c_tile_write_persistent)
        }) {
            self.object_data_bus.unwrap_or(self.tile_data_bus)
        } else if self
            .active_tile_select_write
            .is_some_and(|(old, new)| old != 0 && new == 0 && self.cgb_c_tile_write_persistent)
        {
            self.fetcher.tile_index
        } else {
            vram[self.fetcher.data_address]
        };
        if reset_glitch {
            let lcdc = self.registers.lcdc;
            let data_address = self.fetcher.data_address;
            self.registers.lcdc |= 0x10;
            self.prepare_tile_data_address(false);
            self.object_data_bus = Some(vram[self.fetcher.data_address]);
            self.fetcher.data_address = data_address;
            self.registers.lcdc = lcdc;
        }
        self.active_tile_select_write = None;
        self.cgb_c_tile_write_persistent = false;
        self.last_bg_data_read = Some(BgDataRead::Low);
    }

    fn fetch_data_high(&mut self, vram: &[u8; 0x4000]) {
        let high_glitch = self.cgb_c_high_glitch.take();
        let reset_glitch = high_glitch.is_some_and(|(old, new)| old != 0 && new == 0);
        self.fetcher.high = match high_glitch {
            Some((0, new)) if new != 0 => self.object_data_bus.unwrap_or(self.tile_data_bus),
            Some((old, 0)) if old != 0 => self.fetcher.tile_index,
            _ => match self.active_tile_select_write {
                Some((0, new)) if new != 0 && self.cgb_revision_d => {
                    self.object_data_bus.unwrap_or(self.tile_data_bus)
                }
                Some((old, 0)) if old != 0 && self.cgb_revision_d => self.fetcher.low,
                _ => vram[self.fetcher.data_address],
            },
        };
        if reset_glitch {
            let lcdc = self.registers.lcdc;
            let data_address = self.fetcher.data_address;
            self.registers.lcdc |= 0x10;
            self.prepare_tile_data_address(true);
            self.object_data_bus = Some(vram[self.fetcher.data_address]);
            self.fetcher.data_address = data_address;
            self.registers.lcdc = lcdc;
        }
        self.tile_data_bus = self.fetcher.high;
        self.active_tile_select_write = None;
        self.cgb_c_tile_write_persistent = false;
        self.last_bg_data_read = Some(BgDataRead::High);
    }

    fn fetch_push(&mut self, vram: &[u8; 0x4000]) {
        if !self.window_active
            && let Some(scy) = self.sprite_scy_latch.take()
        {
            let y = scy.wrapping_add(self.ly);
            let map_base = if self.registers.lcdc & 0x08 != 0 {
                0x9C00
            } else {
                0x9800
            };
            self.fetcher.map_address =
                map_base + u16::from(y >> 3) * 32 + u16::from(self.fetcher.tile_column & 31);
            self.fetcher.tile_y = y & 7;
            self.read_tile(vram);
            self.prepare_tile_data_address(false);
            self.fetcher.low = vram[self.fetcher.data_address];
            self.prepare_tile_data_address(true);
            self.fetcher.high = vram[self.fetcher.data_address];
        }
        if self.refetch_push_map {
            self.refetch_push_map(vram);
        }
        self.push_bg_tile();
    }

    fn apply_window_reload(&mut self, vram: &[u8; 0x4000]) {
        let Some(reload) = self.reload_window_tile else {
            return;
        };
        let len = self.bg_fifo.len();
        if len == 0 || len > 8 {
            return;
        }
        if matches!(reload, WindowReload::CopyNextLowToHigh) && len != 8 {
            return;
        }
        self.reload_window_tile = None;
        let lcdc = self.registers.lcdc;
        self.registers.lcdc |= 0x10;
        self.prepare_tile_data_address(false);
        let low = vram[self.fetcher.data_address];
        self.prepare_tile_data_address(true);
        let high = vram[self.fetcher.data_address];
        self.registers.lcdc = lcdc;
        for (index, pixel) in self.bg_fifo.iter_mut().enumerate() {
            let bit = len - 1 - index;
            let low_bit = (low >> bit) & 1;
            let high_bit = (high >> bit) & 1;
            pixel.color = match reload {
                WindowReload::Both => low_bit | (high_bit << 1),
                WindowReload::Low => low_bit | (pixel.color & 2),
                WindowReload::CopyLowToHigh | WindowReload::CopyNextLowToHigh => {
                    let bit = pixel.color & 1;
                    bit | (bit << 1)
                }
            };
        }
    }

    fn advance_scy_write(&mut self) {
        if let Some((countdown, value)) = self.pending_scy.as_mut() {
            *countdown = countdown.saturating_sub(1);
            if *countdown == 0 {
                self.registers.scy = *value;
                self.pending_scy = None;
            }
        }
    }

    fn prepare_tile_address(&mut self) {
        if !self.window_active && self.fetcher.stage_dot == 0 {
            let new_column = self.registers.scx >> 3;
            self.fetcher.tile_column = self
                .fetcher
                .tile_column
                .wrapping_add(new_column.wrapping_sub(self.scx_tile_latch));
            self.scx_tile_latch = new_column;
            if let Some(value) = self.pending_scx_high.take() {
                self.registers.scx = (self.registers.scx & 7) | value;
            }
        }
        let (map_base, tile_row, tile_y) = if self.window_active {
            let map = if self.registers.lcdc & 0x40 != 0 {
                0x9C00
            } else {
                0x9800
            };
            (map, self.window_line >> 3, self.window_line & 7)
        } else {
            let y = self.registers.scy.wrapping_add(self.ly);
            let map = if self.registers.lcdc & 0x08 != 0 {
                0x9C00
            } else {
                0x9800
            };
            (map, y >> 3, y & 7)
        };
        self.fetcher.map_address =
            map_base + u16::from(tile_row) * 32 + u16::from(self.fetcher.tile_column & 31);
        self.fetcher.tile_y = tile_y;
    }

    fn read_tile(&mut self, vram: &[u8; 0x4000]) {
        let map_index = usize::from(self.fetcher.map_address & 0x1FFF);
        self.fetcher.tile_index = vram[map_index];
        self.fetcher.attributes = if self.cgb_game {
            vram[0x2000 + map_index]
        } else {
            0
        };
        self.fetcher.tile_y = if self.cgb_game && self.fetcher.attributes & 0x40 != 0 {
            7 - self.fetcher.tile_y
        } else {
            self.fetcher.tile_y
        };
    }

    fn prepare_tile_data_address(&mut self, high: bool) {
        if !self.cgb_revision_d && !self.window_active {
            self.fetcher.tile_y = self.registers.scy.wrapping_add(self.ly) & 7;
            if self.cgb_game && self.fetcher.attributes & 0x40 != 0 {
                self.fetcher.tile_y = 7 - self.fetcher.tile_y;
            }
        }
        let tile_address = if self.registers.lcdc & 0x10 == 0 {
            0x9000u16.wrapping_add_signed((self.fetcher.tile_index as i8 as i16).wrapping_mul(16))
        } else {
            0x8000 + u16::from(self.fetcher.tile_index) * 16
        };
        let bank = if self.cgb_game {
            usize::from((self.fetcher.attributes >> 3) & 1)
        } else {
            0
        };
        let address = tile_address + u16::from(self.fetcher.tile_y) * 2 + u16::from(high);
        self.fetcher.data_address = bank * 0x2000 + usize::from(address & 0x1FFF);
    }

    fn push_bg_tile(&mut self) {
        let flipped = self.cgb_game && self.fetcher.attributes & 0x20 != 0;
        let palette = if self.cgb_game {
            self.fetcher.attributes & 7
        } else {
            0
        };
        let priority = self.cgb_game && self.fetcher.attributes & 0x80 != 0;
        for x in 0..8 {
            let bit = if flipped { x } else { 7 - x };
            let color = ((self.fetcher.low >> bit) & 1) | (((self.fetcher.high >> bit) & 1) << 1);
            self.bg_fifo.push_back(BgPixel {
                color,
                palette,
                priority,
            });
        }
    }

    fn refetch_push_map(&mut self, vram: &[u8; 0x4000]) {
        self.refetch_push_map = false;
        let map_base = if self.registers.lcdc & 0x08 != 0 {
            0x9C00
        } else {
            0x9800
        };
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
        self.try_activate_window();
    }

    fn try_activate_window(&mut self) {
        let window_x = self
            .window_trigger_at
            .map(i16::from)
            .unwrap_or_else(|| i16::from(self.registers.wx) - 7);
        let can_trigger = !self.window_triggered || self.window_can_retrigger;
        let phase_seven_blocked = !self.cgb_mode
            && self.wx_written
            && self.window_nametable_phase == 7
            && self.registers.wx < 6;
        let window_enabled = if self.window_triggered {
            self.registers.lcdc & 0x20 != 0
        } else {
            self.window_eligible
        };
        if !self.window_active
            && can_trigger
            && !phase_seven_blocked
            && window_enabled
            && self.ly >= self.registers.wy
            && window_x < 160
            && i16::from(self.pixel_x) >= window_x.max(0)
        {
            if self.window_triggered {
                self.window_line = self.window_line.wrapping_add(1);
            }
            self.window_active = true;
            self.window_trigger_at = None;
            self.window_comparator_seen = true;
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
        let reuses_fetch_setup = self
            .next_sprite
            .checked_sub(1)
            .is_some_and(|previous| self.sprites[previous].x == sprite.x);
        self.next_sprite += 1;
        let stall = if reuses_fetch_setup {
            6
        } else {
            self.sprite_stall(&sprite, i16::from(self.pixel_x))
        };
        self.output_stall = stall;
        // The first sprite at an X position pays the BG fetch alignment;
        // consecutive sprites at that X reuse setup and only cost 6 dots.
        self.sprite_extra_dots = self.sprite_extra_dots.saturating_add(stall);
        self.sprite_fetch = Some(SpriteFetch {
            sprite,
            bg_wait: stall - 6,
            advance_bg: true,
            dot: 0,
            low: 0,
            high: 0,
            height: if self.registers.lcdc & 0x04 != 0 {
                16
            } else {
                8
            },
            data_address: 0,
        });
    }

    fn sprite_stall(&self, sprite: &Sprite, pixel_pos: i16) -> u8 {
        let tile_x = (pixel_pos + i16::from(self.registers.scx)) & 7;
        let fetch_wait = (5 - tile_x).max(0) as u8;
        if sprite.x < 0 {
            if sprite.x <= -5 {
                (3 - sprite.x) as u8
            } else if sprite.x == -4 {
                7
            } else {
                6
            }
        } else {
            6 + fetch_wait
        }
    }

    fn step_sprite_fetch(&mut self, vram: &[u8; 0x4000]) {
        if self.sprite_fetch_bg_wait(vram) {
            return;
        }
        self.sprite_fetch_advance_bg(vram);
        if self.sprite_fetch.is_none() {
            return;
        }
        self.update_sprite_fetch_address();
        if !self.read_sprite_fetch_data(vram) {
            return;
        }
        let fetch = self.sprite_fetch.take().expect("sprite fetch is active");
        self.latch_sprite_scy(&fetch);
        self.merge_sprite(fetch);
    }

    fn sprite_fetch_bg_wait(&mut self, vram: &[u8; 0x4000]) -> bool {
        if self
            .sprite_fetch
            .as_ref()
            .is_none_or(|fetch| fetch.bg_wait == 0)
        {
            return false;
        }
        if self.sprite_fetch.as_ref().unwrap().advance_bg {
            self.step_bg_fetcher(vram);
        }
        self.sprite_fetch.as_mut().unwrap().bg_wait -= 1;
        true
    }

    fn sprite_fetch_advance_bg(&mut self, vram: &[u8; 0x4000]) {
        if self
            .sprite_fetch
            .as_ref()
            .is_some_and(|fetch| fetch.advance_bg && fetch.dot < 2)
            && !self.window_active
        {
            self.step_bg_fetcher(vram);
        }
    }

    fn update_sprite_fetch_address(&mut self) {
        let Some(fetch) = self.sprite_fetch.as_mut() else {
            return;
        };
        let relatch_high =
            fetch.dot == 4 && (self.registers.scx != 0 || self.relatch_obj_size_high);
        if fetch.dot == 2 || relatch_high {
            fetch.height = if self.registers.lcdc & 0x04 != 0 {
                16
            } else {
                8
            };
        }
        if fetch.dot == 4 {
            self.relatch_obj_size_high = false;
        }
        if fetch.dot == 2 || fetch.dot == 4 {
            let address = sprite_fetch_data_address(fetch, self.ly, self.cgb_mode);
            fetch.data_address = address;
        }
    }

    fn read_sprite_fetch_data(&mut self, vram: &[u8; 0x4000]) -> bool {
        let Some(fetch) = self.sprite_fetch.as_mut() else {
            return true;
        };
        if fetch.dot == 3 || fetch.dot == 5 {
            let value = vram[fetch.data_address];
            if fetch.dot == 3 {
                fetch.low = value
            } else {
                fetch.high = value;
                self.tile_data_bus = value;
                self.object_data_bus = Some(value);
            }
        }
        fetch.dot += 1;
        fetch.dot >= 6
    }

    fn latch_sprite_scy(&mut self, fetch: &SpriteFetch) {
        self.sprite_scy_latch =
            (self.cgb_revision_d && fetch.sprite.x < 0 && !self.window_active && self.scy_written)
                .then(|| {
                    if fetch.sprite.x == -8 {
                        self.pending_scy
                            .map(|(_, value)| value)
                            .unwrap_or(self.registers.scy)
                    } else {
                        self.registers.scy
                    }
                });
    }

    fn merge_sprite(&mut self, fetch: SpriteFetch) {
        for pixel in 0..8i16 {
            self.merge_sprite_pixel(&fetch, pixel);
        }
    }

    fn merge_sprite_pixel(&mut self, fetch: &SpriteFetch, pixel: i16) {
        let screen_x = fetch.sprite.x + pixel;
        if !(0..160).contains(&screen_x) {
            return;
        }
        let tile_x = if fetch.sprite.flags & 0x20 != 0 {
            pixel as u8
        } else {
            7 - pixel as u8
        };
        let color = ((fetch.low >> tile_x) & 1) | (((fetch.high >> tile_x) & 1) << 1);
        if color == 0 {
            return;
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
            Self::cgb_color(palettes[usize::from(pixel.palette) * 4 + usize::from(pixel.color)])
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
            0xFF40 => self.write_lcdc(value),
            0xFF42 => {
                self.scy_written = true;
                if self.cgb_revision_d {
                    self.pending_scy = Some((4, value));
                } else {
                    self.pending_scy = Some((3, value));
                }
            }
            0xFF43 => self.write_scx(value),
            0xFF47 => self.write_bgp(value),
            0xFF48 if self.cgb_mode && !self.cgb_revision_d => {
                self.pending_obp = Some((1, 0, value));
            }
            0xFF49 if self.cgb_mode && !self.cgb_revision_d => {
                self.pending_obp = Some((1, 1, value));
            }
            0xFF48 => self.registers.obp0 = value,
            0xFF49 => self.registers.obp1 = value,
            0xFF4A => self.registers.wy = value,
            0xFF4B => self.write_wx(value),
            _ => {}
        }
    }

    fn write_lcdc(&mut self, value: u8) {
        let old = self.registers.lcdc;
        let old_written = self.written_lcdc;
        self.written_lcdc = value;
        let changed = old_written ^ value;
        let defer_obj_size_set = value & 4 != 0
            && self.registers.scx == 0
            && self
                .sprite_fetch
                .as_ref()
                .is_some_and(|fetch| fetch.dot == 2);
        let obj_size_delay = if changed & 4 != 0 && (value & 4 == 0 || defer_obj_size_set) {
            self.sprite_fetch
                .as_ref()
                .and_then(|fetch| (1..=2).contains(&fetch.dot).then(|| 3 - fetch.dot))
        } else {
            None
        };
        let defer_obj_size = obj_size_delay.is_some();
        let preserve = 0x5B | if defer_obj_size { 4 } else { 0 };
        self.registers.lcdc = (value & !preserve) | (old & preserve);

        if changed & 1 != 0 {
            self.apply_bg_enable(value);
        }
        if changed & 2 != 0 {
            self.apply_obj_enable(value);
        }
        if let Some(delay) = obj_size_delay {
            self.apply_obj_size_delay(value, delay, defer_obj_size_set);
        }
        if changed & 0x08 != 0 {
            self.apply_bg_map_change(value);
        }
        if changed & 0x40 != 0 {
            self.apply_window_map_change(value);
        }
        if changed & 0x10 != 0 {
            self.apply_tile_select_change(value, old_written);
        }
        if old_written & 0x20 != 0 && value & 0x20 == 0 {
            self.apply_window_disable();
        }
    }

    fn apply_bg_enable(&mut self, value: u8) {
        let object_x = self.next_sprite.checked_sub(1).map(|i| self.sprites[i].x);
        let immediate_dmg_disable = !self.cgb_mode
            && value & 1 == 0
            && self.pixel_x == 0
            && object_x == Some(-6);
        let delay = if immediate_dmg_disable {
            0
        } else if !self.cgb_mode {
            1
        } else if !self.cgb_revision_d {
            2
        } else if self.fetcher.stage == FetchStage::Tile && self.fetcher.stage_dot == 0 {
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

    fn apply_obj_enable(&mut self, value: u8) {
        let object_x = self.next_sprite.checked_sub(1).map(|i| self.sprites[i].x);
        if !self.cgb_mode
            && value & 2 == 0
            && self.sprite_fetch.as_ref().is_some_and(|fetch| fetch.dot == 0)
        {
            self.sprite_fetch = None;
            self.sprite_extra_dots = self.sprite_extra_dots.saturating_sub(self.output_stall);
            self.output_stall = 0;
        }
        if !self.cgb_mode && value & 2 == 0 && self.pixel_x == 0 && object_x == Some(-6) {
            self.registers.lcdc &= !2;
            self.pending_obj_enable = None;
        } else if !self.cgb_mode {
            self.pending_obj_enable = Some((1, value & 2));
        } else if self.output_stall >= 2 {
            self.registers.lcdc = (self.registers.lcdc & !2) | (value & 2);
            self.pending_obj_enable = None;
        } else {
            self.pending_obj_enable = Some((2, value & 2));
        }
    }

    fn apply_obj_size_delay(&mut self, value: u8, delay: u8, defer_set: bool) {
        self.pending_obj_size = Some((delay, value & 4));
        self.relatch_obj_size_high = defer_set;
    }

    fn apply_bg_map_change(&mut self, value: u8) {
        if let Some(fetch) = self.sprite_fetch.as_mut() {
            fetch.advance_bg = false;
        }
        let last_object_x = self.next_sprite.checked_sub(1).map(|i| self.sprites[i].x);
        if !self.cgb_mode && matches!(last_object_x, Some(-7 | -6)) {
            let delay = if last_object_x == Some(-7) { 3 } else { 2 };
            self.pending_map_select = Some((delay, value & 0x08));
            self.refetch_push_map = true;
            return;
        }
        if !self.cgb_mode && last_object_x.is_none_or(|x| x >= 0) {
            self.pending_map_select = Some((2, value & 0x08));
            return;
        }
        let immediate = self.output_stall >= 2
            && (last_object_x == Some(-8) || self.fetcher.stage == FetchStage::Tile);
        if immediate {
            self.registers.lcdc = (self.registers.lcdc & !0x48) | (value & 0x48);
            self.pending_map_select = None;
            self.refetch_push_map = last_object_x == Some(-8)
                && self.fetcher.stage == FetchStage::Push
                && !self.window_active;
        } else {
            self.pending_map_select = Some((4, value & 0x48));
        }
    }

    fn apply_window_map_change(&mut self, value: u8) {
        if let Some(fetch) = self.sprite_fetch.as_mut() {
            fetch.advance_bg = false;
        }
        let object_x = self.next_sprite.checked_sub(1).map(|i| self.sprites[i].x);
        let initial_offscreen_set = !self.window_active
            && value & 0x40 != 0
            && object_x == Some(-8)
            && self.output_stall >= 2;
        let immediate_active = self.window_active
            && self.output_stall < 2
            && (self.cgb_revision_d && object_x.is_none_or(|x| x >= 0)
                || !self.cgb_revision_d && self.cgb_mode && object_x == Some(0));
        if initial_offscreen_set || immediate_active {
            self.registers.lcdc = (self.registers.lcdc & !0x40) | (value & 0x40);
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

    fn apply_tile_select_change(&mut self, value: u8, old_written: u8) {
        let old_tile_select = old_written & 0x10;
        let new_tile_select = value & 0x10;
        self.apply_window_tile_select(old_tile_select, new_tile_select);
        let collided = self.detect_tile_select_collision(old_tile_select, new_tile_select);
        self.pending_tile_select_write = (!collided).then_some((old_tile_select, new_tile_select));
        self.apply_cgb_c_tile_select_glitches(old_tile_select, new_tile_select);
        self.pending_tile_select = Some((3, value & 0x10));
    }

    fn apply_window_tile_select(&mut self, old: u8, new: u8) {
        if !(self.cgb_revision_d && self.window_active && old != 0 && new == 0) {
            return;
        }
        let object_x = self
            .next_sprite
            .checked_sub(1)
            .map(|index| self.sprites[index].x);
        if object_x == Some(-7) {
            self.correct_last_bg_color(|color| color | 1);
        } else if object_x == Some(0) && self.fetcher.tile_y == 0 {
            self.force_bg_high_delay = 8;
            self.force_bg_high_pixels = 8;
        } else if object_x == Some(9) {
            self.correct_last_bg_color(|color| color & 1);
        }
        self.reload_window_tile = self
            .next_sprite
            .checked_sub(1)
            .map(|index| self.sprites[index].x)
            .and_then(|x| match x {
                -8 => Some(WindowReload::Both),
                -7 => Some(WindowReload::Low),
                7 => Some(WindowReload::CopyNextLowToHigh),
                8..=9 => Some(WindowReload::CopyLowToHigh),
                _ => None,
            });
    }

    fn detect_tile_select_collision(&mut self, old: u8, new: u8) -> bool {
        self.cgb_revision_d
            && match (old, new, self.last_bg_data_read) {
                (0, new, Some(BgDataRead::Low)) if new != 0 => {
                    self.fetcher.low = self.tile_data_bus;
                    true
                }
                (0, new, Some(BgDataRead::High)) if new != 0 => {
                    self.fetcher.high = self.tile_data_bus;
                    true
                }
                (old, 0, Some(BgDataRead::High)) if old != 0 => {
                    self.fetcher.high = self.fetcher.low;
                    true
                }
                _ => false,
            }
    }

    fn apply_cgb_c_tile_select_glitches(&mut self, old: u8, new: u8) {
        self.cgb_c_tile_write_persistent = !self.cgb_revision_d
            && self.fetcher.stage == FetchStage::Tile
            && self.fetcher.stage_dot == 0
            && (self.window_active || (old != 0 && new == 0));
        if !self.cgb_revision_d
            && !self.window_active
            && old != 0
            && new == 0
            && self.fetcher.stage == FetchStage::Tile
            && self.fetcher.stage_dot == 0
            && self
                .next_sprite
                .checked_sub(1)
                .map(|index| self.sprites[index].x)
                == Some(5)
        {
            let low = self.object_data_bus.unwrap_or(self.tile_data_bus);
            self.correct_last_bg_color(|color| (color & 2) | ((low >> 7) & 1));
            self.force_bg_low = low;
            self.force_bg_low_pixels = 7;
        }
        self.cgb_c_high_glitch = (!self.cgb_revision_d
            && self.fetcher.stage == FetchStage::DataLow
            && self.fetcher.stage_dot == 0
            && self.bg_fifo.len() == 5)
            .then_some((old, new));
    }

    fn apply_window_disable(&mut self) {
        if self.window_active {
            let pixels_left = 8 - (self.window_pixels & 7);
            self.window_disable_countdown = Some(pixels_left + 8);
        }
    }

    fn write_scx(&mut self, value: u8) {
        if self.ly != 0 && self.pixel_x == 0 && self.fetcher.stage == FetchStage::Tile {
            self.fine_discard = value & 7;
        }
        let object_x = self.next_sprite.checked_sub(1).map(|i| self.sprites[i].x);
        let cgb_c_sprite_phase = self.cgb_mode
            && !self.cgb_revision_d
            && ((self.fetcher.stage == FetchStage::Push
                && self.bg_fifo.len() == 2
                && object_x == Some(0))
                || (self.fetcher.stage == FetchStage::Sleep
                    && self.output_stall == 2
                    && matches!((self.bg_fifo.len(), object_x), (8, Some(8)) | (7, Some(9)))));
        let defer_high = if !self.cgb_mode {
            self.fetcher.stage == FetchStage::Tile && self.fetcher.stage_dot == 0
        } else if self.cgb_revision_d || cgb_c_sprite_phase {
            (self.fetcher.stage == FetchStage::Push && self.bg_fifo.len() <= 1)
                || (self.fetcher.stage == FetchStage::Tile && self.fetcher.stage_dot == 0)
        } else {
            matches!(self.fetcher.stage, FetchStage::Sleep | FetchStage::Push)
                || (self.fetcher.stage == FetchStage::Tile && self.fetcher.stage_dot == 0)
        };
        if defer_high {
            self.pending_scx_high = Some(value & 0xF8);
            self.registers.scx = (self.registers.scx & 0xF8) | (value & 7);
        } else {
            self.registers.scx = value;
        }
    }

    fn write_bgp(&mut self, value: u8) {
        if self.ly != 0
            && self.registers.wx == 0
            && self.registers.scx & 7 == 0
            && self.registers.lcdc & 0x20 != 0
            && self.window_eligible
            && !self.wx_written
            && value != 0
        {
            let delay = if self.cgb_mode && !self.cgb_revision_d {
                8
            } else {
                7
            };
            self.pending_bgp = Some((delay, value));
        } else if (self.cgb_mode && !self.cgb_revision_d)
            || (self.ly == 0
                && self.window_active
                && self.registers.wx == 0
                && !self.wx_written)
        {
            let stable_line_zero_window = self.ly == 0
                && self.window_active
                && self.registers.wx == 0
                && !self.wx_written;
            let delay = if self.cgb_mode && !self.cgb_revision_d && stable_line_zero_window {
                2
            } else {
                1
            };
            self.pending_bgp = Some((delay, value));
        } else {
            self.registers.bgp = value;
            self.pending_bgp = None;
        }
    }

    fn write_wx(&mut self, value: u8) {
        let old_window_x = i16::from(self.registers.wx) - 7;
        let pixel_x = i16::from(self.pixel_x);
        if self.window_comparator_seen
            && !self.window_active
            && (pixel_x..=pixel_x + 1).contains(&old_window_x)
        {
            self.window_trigger_at = Some(old_window_x as u8);
        }
        self.wx_written = true;
        self.window_zero_at = None;
        self.registers.wx = value;
        if self.window_comparator_seen {
            self.window_can_retrigger = true;
            let window_x = i16::from(value) - 7;
            if window_x > i16::from(self.pixel_x)
                && self.window_nametable_phase != 7
                && value.saturating_sub(7) & 7 == self.window_nametable_phase
            {
                self.window_zero_at = Some(value.saturating_sub(7));
            }
            if !self.cgb_mode
                && self.window_nametable_phase == 7
                && !self.window_active
                && self.window_trigger_at.is_none()
                && window_x >= 0
                && window_x < i16::from(self.pixel_x)
            {
                self.window_triggered = true;
                self.window_can_retrigger = false;
                self.window_zero_at = None;
            }
        }
    }

    pub(super) fn complete(&self) -> bool {
        self.complete
    }

    pub(super) fn sprite_extra_dots(&self) -> u8 {
        self.sprite_extra_dots
    }

    pub(super) fn unstarted_visible_sprite_pending(&self) -> bool {
        self.next_sprite < self.sprites.len() && self.sprites[self.next_sprite].x < 160
    }

    fn correct_last_bg_color(&mut self, update: impl FnOnce(u8) -> u8) {
        let Some((x, color, candidates)) = self.last_output else {
            return;
        };
        let corrected = update(color) & 3;
        self.last_output = Some((x, corrected, candidates));
        self.corrected_output = Some(OutputPixel {
            x,
            color: candidates[usize::from(corrected)],
        });
    }

    pub(super) fn take_corrected_output(&mut self) -> Option<OutputPixel> {
        self.corrected_output.take()
    }

    pub(super) fn final_window_line(&self) -> u8 {
        self.window_line.wrapping_add(u8::from(self.window_seen))
    }
}

fn sprite_fetch_data_address(fetch: &SpriteFetch, ly: u8, cgb_mode: bool) -> usize {
    let mut tile_y = (i16::from(ly) - fetch.sprite.y) as u8 & (fetch.height - 1);
    if fetch.sprite.flags & 0x40 != 0 {
        tile_y ^= fetch.height - 1;
    }
    let tile = if fetch.height == 16 {
        (fetch.sprite.tile & 0xFE) | u8::from(tile_y >= 8)
    } else {
        fetch.sprite.tile
    };
    let bank = if cgb_mode {
        usize::from((fetch.sprite.flags >> 3) & 1)
    } else {
        0
    };
    let address = 0x8000u16
        + u16::from(tile) * 16
        + u16::from(tile_y & 7) * 2
        + u16::from(fetch.dot == 4);
    bank * 0x2000 + usize::from(address & 0x1FFF)
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
        for stage in [FetchStage::Tile, FetchStage::DataLow, FetchStage::DataHigh] {
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
            Mode3Pipeline::new(registers(), 0, 0, false, Vec::new(), false, false, true, 0);
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

    #[test]
    fn only_unstarted_visible_sprites_hold_pixel_transfer_open() {
        let sprite = |x| Sprite {
            x,
            tile: 0,
            y: 0,
            flags: 0,
            oam_index: 0,
        };
        let visible = Mode3Pipeline::new(
            registers(),
            0,
            0,
            false,
            vec![sprite(159)],
            true,
            true,
            true,
            0,
        );
        let offscreen = Mode3Pipeline::new(
            registers(),
            0,
            0,
            false,
            vec![sprite(160)],
            true,
            true,
            true,
            0,
        );

        assert!(visible.unstarted_visible_sprite_pending());
        assert!(!offscreen.unstarted_visible_sprite_pending());
    }

    #[test]
    fn dmg_bg_map_change_relatches_during_clipped_sprite_fetch() {
        for (x, expected_delay) in [(-7, 3), (-6, 2)] {
            let sprite = Sprite {
                x,
                tile: 0,
                y: 0,
                flags: 0,
                oam_index: 0,
            };
            let mut pipeline = Mode3Pipeline::new(
                registers(),
                0,
                0,
                false,
                vec![sprite],
                false,
                false,
                true,
                0,
            );
            pipeline.next_sprite = 1;
            pipeline.sprite_fetch = Some(SpriteFetch {
                sprite,
                bg_wait: 0,
                advance_bg: true,
                dot: 0,
                low: 0,
                high: 0,
                height: 8,
                data_address: 0,
            });

            pipeline.apply_bg_map_change(0x08);

            assert_eq!(pipeline.pending_map_select, Some((expected_delay, 0x08)));
            assert!(pipeline.refetch_push_map);
        }
    }

    #[test]
    fn cgb_c_bgp_write_advances_during_sprite_stall() {
        let mut pipeline = Mode3Pipeline::new(
            registers(),
            0,
            0,
            false,
            Vec::new(),
            true,
            true,
            false,
            0,
        );
        pipeline.startup_dots = 0;
        pipeline.output_stall = 2;
        pipeline.write_bgp(0x1B);

        assert_eq!(pipeline.registers.bgp, 0xE4);
        pipeline.step(&[0; 0x4000], &[0; 32], &[0; 32]);
        assert_eq!(pipeline.registers.bgp, 0x1B);
    }

    #[test]
    fn line_zero_wx_zero_bgp_latch_requires_stable_wx() {
        let mut pipeline = Mode3Pipeline::new(
            registers(),
            0,
            0,
            true,
            Vec::new(),
            false,
            false,
            true,
            0,
        );
        pipeline.window_active = true;
        pipeline.registers.wx = 0;
        pipeline.write_bgp(0x1B);
        assert_eq!(pipeline.pending_bgp, Some((1, 0x1B)));

        pipeline.pending_bgp = None;
        pipeline.wx_written = true;
        pipeline.write_bgp(0x2D);
        assert_eq!(pipeline.pending_bgp, None);
        assert_eq!(pipeline.registers.bgp, 0x2D);
    }

    #[test]
    fn cgb_c_obj_palette_write_latches_after_one_dot() {
        let mut pipeline = Mode3Pipeline::new(
            registers(),
            0,
            0,
            false,
            Vec::new(),
            true,
            true,
            false,
            0,
        );
        pipeline.startup_dots = 0;
        pipeline.output_stall = 1;
        pipeline.write_register(0xFF48, 0x1B);
        assert_eq!(pipeline.registers.obp0, 0xE4);

        pipeline.step(&[0; 0x4000], &[0; 32], &[0; 32]);
        assert_eq!(pipeline.registers.obp0, 0x1B);
    }

    #[test]
    fn cgb_c_wx_zero_aligned_bgp_quirk_takes_eight_dots() {
        let mut regs = registers();
        regs.lcdc |= 0x20;
        regs.wx = 0;
        let mut pipeline = Mode3Pipeline::new(
            regs,
            8,
            0,
            true,
            Vec::new(),
            true,
            true,
            false,
            0,
        );
        pipeline.write_bgp(0x1B);
        assert_eq!(pipeline.pending_bgp, Some((8, 0x1B)));
    }

    #[test]
    fn cgb_c_lcdc_enable_latch_advances_during_stalls() {
        for (revision_d, expected_countdown) in [(false, 1), (true, 2)] {
            let mut pipeline = Mode3Pipeline::new(
                registers(),
                0,
                0,
                false,
                Vec::new(),
                true,
                true,
                revision_d,
                0,
            );
            pipeline.startup_dots = 0;
            pipeline.output_stall = 1;
            pipeline.pending_bg_enable = Some((2, 0));
            pipeline.step(&[0; 0x4000], &[0; 32], &[0; 32]);
            assert_eq!(
                pipeline.pending_bg_enable.map(|(countdown, _)| countdown),
                Some(expected_countdown)
            );
        }
    }

    #[test]
    fn disabling_inactive_window_preserves_future_trigger() {
        let mut pipeline = Mode3Pipeline::new(
            registers(),
            0,
            0,
            true,
            Vec::new(),
            false,
            false,
            true,
            0,
        );
        pipeline.apply_window_disable();
        assert!(!pipeline.window_triggered);
        assert!(!pipeline.window_seen);
        assert_eq!(pipeline.window_disable_countdown, None);
    }

    #[test]
    fn cgb_c_window_map_change_is_immediate_at_sprite_x_zero() {
        let sprite = Sprite {
            x: 0,
            tile: 0,
            y: 0,
            flags: 0,
            oam_index: 0,
        };
        let mut pipeline = Mode3Pipeline::new(
            registers(),
            0,
            0,
            true,
            vec![sprite],
            true,
            true,
            false,
            0,
        );
        pipeline.window_active = true;
        pipeline.next_sprite = 1;
        pipeline.apply_window_map_change(0x40);
        assert_eq!(pipeline.registers.lcdc & 0x40, 0x40);
        assert_eq!(pipeline.pending_map_select, None);
    }

    #[test]
    fn cgb_c_scx_high_bits_are_immediate_at_sprite_x_zero_push() {
        let sprite = Sprite {
            x: 0,
            tile: 0,
            y: 0,
            flags: 0,
            oam_index: 0,
        };
        let mut pipeline = Mode3Pipeline::new(
            registers(),
            64,
            0,
            false,
            vec![sprite],
            true,
            true,
            false,
            0,
        );
        pipeline.next_sprite = 1;
        pipeline.fetcher.stage = FetchStage::Push;
        pipeline.bg_fifo.push_back(BgPixel::default());
        pipeline.bg_fifo.push_back(BgPixel::default());
        pipeline.write_scx(0x20);

        assert_eq!(pipeline.registers.scx, 0x20);
        assert_eq!(pipeline.pending_scx_high, None);
    }

    #[test]
    fn dmg_obj_disable_cancels_sprite_fetch_at_dot_zero() {
        let sprite = Sprite {
            x: 8,
            tile: 0,
            y: 0,
            flags: 0,
            oam_index: 0,
        };
        let mut pipeline = Mode3Pipeline::new(
            registers(),
            0,
            0,
            false,
            vec![sprite],
            false,
            false,
            true,
            0,
        );
        pipeline.next_sprite = 1;
        pipeline.output_stall = 6;
        pipeline.sprite_extra_dots = 6;
        pipeline.sprite_fetch = Some(SpriteFetch {
            sprite,
            bg_wait: 0,
            advance_bg: true,
            dot: 0,
            low: 0,
            high: 0,
            height: 8,
            data_address: 0,
        });
        pipeline.apply_obj_enable(0);

        assert!(pipeline.sprite_fetch.is_none());
        assert_eq!(pipeline.output_stall, 0);
        assert_eq!(pipeline.sprite_extra_dots, 0);
        assert_eq!(pipeline.pending_obj_enable, Some((1, 0)));
    }

    #[test]
    fn dmg_bg_disable_is_immediate_at_clipped_x_minus_six() {
        let sprite = Sprite {
            x: -6,
            tile: 0,
            y: 0,
            flags: 0,
            oam_index: 0,
        };
        let mut pipeline = Mode3Pipeline::new(
            registers(),
            0,
            0,
            false,
            vec![sprite],
            false,
            false,
            true,
            0,
        );
        pipeline.next_sprite = 1;
        pipeline.apply_bg_enable(0);
        assert_eq!(pipeline.registers.lcdc & 1, 0);
        assert_eq!(pipeline.pending_bg_enable, None);
    }

    #[test]
    fn dmg_oam_wx_four_seeds_first_dot_window_activation() {
        let mut regs = registers();
        regs.lcdc |= 0x20;
        regs.wy = 4;
        regs.wx = 4;
        let mut pipeline = Mode3Pipeline::new(
            regs,
            4,
            0,
            true,
            Vec::new(),
            false,
            false,
            true,
            0,
        );
        pipeline.set_wx_written_during_oam(true);
        assert!(pipeline.window_activation_pending);

        pipeline.step(&[0; 0x4000], &[0; 32], &[0; 32]);
        assert!(pipeline.window_active);
        assert!(!pipeline.window_activation_pending);
    }

    #[test]
    fn window_nametable_collision_inserts_without_consuming_fifo() {
        let mut pipeline = Mode3Pipeline::new(
            registers(),
            0,
            0,
            false,
            Vec::new(),
            false,
            false,
            true,
            0,
        );
        pipeline.bg_fifo.push_back(BgPixel {
            color: 1,
            palette: 0,
            priority: false,
        });
        pipeline.window_zero_at = Some(0);

        assert!(pipeline.emit_output_pixel(&[0; 32], &[0; 32]).is_some());
        assert_eq!(pipeline.bg_fifo.len(), 1);
        assert_eq!(pipeline.window_zero_at, None);
    }

    #[test]
    fn exact_wx_coordinate_does_not_imply_nametable_collision() {
        let mut pipeline = Mode3Pipeline::new(
            registers(),
            0,
            0,
            false,
            Vec::new(),
            false,
            false,
            true,
            0,
        );
        pipeline.window_seen = true;
        pipeline.window_triggered = true;
        pipeline.write_wx(7);
        assert_eq!(pipeline.window_zero_at, None);
    }

    #[test]
    fn oam_wx_five_moves_nametable_collision_to_phase_six() {
        let mut regs = registers();
        regs.wx = 5;
        let mut pipeline = Mode3Pipeline::new(
            regs,
            12,
            0,
            true,
            Vec::new(),
            false,
            false,
            true,
            0,
        );
        pipeline.set_wx_written_during_oam(true);
        pipeline.window_comparator_seen = true;
        pipeline.window_triggered = true;
        pipeline.write_wx(13);

        assert_eq!(pipeline.window_nametable_phase, 6);
        assert_eq!(pipeline.window_zero_at, Some(6));
    }

    #[test]
    fn phase_seven_does_not_render_before_dynamic_wx_six() {
        let mut regs = registers();
        regs.lcdc |= 0x20;
        regs.wx = 6;
        let mut pipeline = Mode3Pipeline::new(
            regs,
            4,
            0,
            true,
            Vec::new(),
            false,
            false,
            true,
            0,
        );
        pipeline.set_wx_written_during_oam(true);
        pipeline.window_comparator_seen = true;
        pipeline.write_wx(4);
        pipeline.try_activate_window();

        assert!(!pipeline.window_active);
        assert!(!pipeline.window_seen);
        assert_eq!(pipeline.final_window_line(), 0);
    }

    #[test]
    fn wx_write_preserves_one_pixel_comparator_lookahead() {
        let mut regs = registers();
        regs.wx = 101;
        let mut pipeline = Mode3Pipeline::new(
            regs,
            101,
            0,
            true,
            Vec::new(),
            false,
            false,
            true,
            0,
        );
        pipeline.window_comparator_seen = true;
        pipeline.window_nametable_phase = 7;
        pipeline.pixel_x = 93;
        pipeline.write_wx(80);
        assert_eq!(pipeline.window_trigger_at, Some(94));

        pipeline.pixel_x = 94;
        pipeline.try_activate_window();
        assert!(pipeline.window_active);
        assert_eq!(pipeline.window_trigger_at, None);
    }

    #[test]
    fn wx_write_replaces_trigger_beyond_comparator_lookahead() {
        let mut regs = registers();
        regs.wx = 102;
        let mut pipeline = Mode3Pipeline::new(
            regs,
            102,
            0,
            true,
            Vec::new(),
            false,
            false,
            true,
            0,
        );
        pipeline.window_comparator_seen = true;
        pipeline.window_nametable_phase = 7;
        pipeline.pixel_x = 93;
        pipeline.write_wx(80);

        assert_eq!(pipeline.window_trigger_at, None);
        assert!(pipeline.window_triggered);
        assert!(!pipeline.window_can_retrigger);
    }

    #[test]
    fn dmg_scx_high_bits_are_immediate_during_push() {
        let mut pipeline = Mode3Pipeline::new(
            registers(),
            8,
            0,
            false,
            Vec::new(),
            false,
            false,
            true,
            0,
        );
        pipeline.fetcher.stage = FetchStage::Push;
        pipeline.bg_fifo.push_back(BgPixel::default());
        pipeline.write_scx(0x20);

        assert_eq!(pipeline.registers.scx, 0x20);
        assert_eq!(pipeline.pending_scx_high, None);
    }
}
