#[derive(Debug)]
pub(super) struct Mode3Timing {
    pixel_x: u8,
    startup_dots: u8,
    stall_dots: u8,
    fine_scroll_x: u8,
    sprite_x: Vec<i16>,
    next_sprite: usize,
    last_sprite_tile: Option<i16>,
    window_started: bool,
    window_seen: bool,
    complete: bool,
}

impl Mode3Timing {
    pub(super) fn new(cgb_mode: bool, scx: u8, sprite_x: Vec<i16>) -> Self {
        Self {
            pixel_x: 0,
            startup_dots: if cgb_mode { 19 } else { 18 } + (scx & 7),
            stall_dots: 0,
            fine_scroll_x: scx & 7,
            sprite_x,
            next_sprite: 0,
            last_sprite_tile: None,
            window_started: false,
            window_seen: false,
            complete: false,
        }
    }

    pub(super) fn step(&mut self, lcdc: u8, scx: u8, ly: u8, wy: u8, wx: u8) {
        if self.complete {
            return;
        }
        if self.startup_dots != 0 {
            if lcdc & 0x20 != 0 && ly >= wy && wx <= 7 {
                self.window_seen = true;
            }
            self.startup_dots -= 1;
            return;
        }
        if self.stall_dots != 0 {
            self.stall_dots -= 1;
            return;
        }

        let window_x = i16::from(wx) - 7;
        if !self.window_started
            && lcdc & 0x20 != 0
            && ly >= wy
            && window_x < 160
            && i16::from(self.pixel_x) >= window_x.max(0)
        {
            self.window_started = true;
            self.window_seen = true;
            self.stall_dots = 5;
            return;
        }

        if lcdc & 0x02 == 0 {
            while self.next_sprite < self.sprite_x.len()
                && self.sprite_x[self.next_sprite] <= i16::from(self.pixel_x)
            {
                self.next_sprite += 1;
            }
        }

        while self.next_sprite < self.sprite_x.len()
            && self.sprite_x[self.next_sprite] <= i16::from(self.pixel_x)
        {
            let sprite_x = self.sprite_x[self.next_sprite];
            let tile = (i16::from(self.pixel_x) + i16::from(scx)) / 8;
            let fetch_wait = if sprite_x == -8 {
                5
            } else if self.last_sprite_tile == Some(tile) {
                0
            } else {
                let tile_x = (i16::from(self.pixel_x) + i16::from(scx)) & 7;
                (5 - tile_x).max(0) as u8
            };
            self.last_sprite_tile = Some(tile);
            self.stall_dots = self.stall_dots.saturating_add(6 + fetch_wait);
            self.next_sprite += 1;
        }
        if self.stall_dots != 0 {
            self.stall_dots -= 1;
            return;
        }

        self.pixel_x += 1;
        self.complete = self.pixel_x == 160;
    }

    pub(super) fn latch_pixel(&self, register: u16, old_value: u8, value: u8) -> u8 {
        let pixel_x = match register {
            0xFF40 if (old_value ^ value) & 0x40 != 0 => self.pixel_x.saturating_add(7) & !7,
            _ => self.pixel_x,
        };
        pixel_x.min(159)
    }

    pub(super) fn fine_scroll_x(&self) -> u8 {
        self.fine_scroll_x
    }

    pub(super) fn window_seen(&self) -> bool {
        self.window_seen
    }

    pub(super) fn complete(&self) -> bool {
        self.complete
    }
}
