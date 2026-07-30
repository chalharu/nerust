#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FetchStage {
    Tile,
    DataLow,
    DataHigh,
    Push,
}

#[derive(Debug)]
pub(super) struct Mode3Timing {
    pixel_x: u8,
    startup_dots: u8,
    stall_dots: u8,
    fetch_dot: u8,
    fetch_pixel_x: u8,
    fine_scroll_x: u8,
    sprite_x: Vec<i16>,
    next_sprite: usize,
    last_sprite_tile: Option<i16>,
    window_active: bool,
    window_seen: bool,
    window_triggered: bool,
    window_can_retrigger: bool,
    window_disable_pending: bool,
    window_pixels: u8,
    complete: bool,
}

impl Mode3Timing {
    pub(super) fn new(cgb_mode: bool, scx: u8, sprite_x: Vec<i16>) -> Self {
        Self {
            pixel_x: 0,
            startup_dots: if cgb_mode { 19 } else { 18 } + (scx & 7),
            stall_dots: 0,
            fetch_dot: 0,
            fetch_pixel_x: 0,
            fine_scroll_x: scx & 7,
            sprite_x,
            next_sprite: 0,
            last_sprite_tile: None,
            window_active: false,
            window_seen: false,
            window_triggered: false,
            window_can_retrigger: false,
            window_disable_pending: false,
            window_pixels: 0,
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
            self.advance_fetcher();
            return;
        }
        if self.stall_dots != 0 {
            self.stall_dots -= 1;
            return;
        }

        let window_x = i16::from(wx) - 7;
        if !self.window_active
            && (!self.window_triggered || self.window_can_retrigger)
            && lcdc & 0x20 != 0
            && ly >= wy
            && window_x < 160
            && i16::from(self.pixel_x) >= window_x.max(0)
        {
            self.window_active = true;
            self.window_seen = true;
            self.window_triggered = true;
            self.window_can_retrigger = false;
            self.window_pixels = if window_x < 0 { (-window_x) as u8 } else { 0 };
            self.stall_dots = 5;
            self.fetch_dot = 0;
            self.fetch_pixel_x = self.pixel_x;
            return;
        }

        if self.window_active && self.window_disable_pending && self.window_pixels & 7 == 0 {
            self.window_active = false;
            self.window_disable_pending = false;
            self.stall_dots = 5;
            self.fetch_dot = 0;
            self.fetch_pixel_x = self.pixel_x;
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
        if self.window_active {
            self.window_pixels = self.window_pixels.wrapping_add(1);
        }
        self.complete = self.pixel_x == 160;
        self.advance_fetcher();
    }

    fn advance_fetcher(&mut self) {
        self.fetch_dot = (self.fetch_dot + 1) & 7;
        if self.fetch_dot == 0 {
            self.fetch_pixel_x = self.fetch_pixel_x.saturating_add(8);
        }
    }

    pub(super) fn fetch_stage(&self) -> FetchStage {
        match self.fetch_dot {
            0 | 1 => FetchStage::Tile,
            2 | 3 => FetchStage::DataLow,
            4 | 5 => FetchStage::DataHigh,
            _ => FetchStage::Push,
        }
    }

    pub(super) fn latch_pixel(&mut self, register: u16, old_value: u8, value: u8, ly: u8) -> u8 {
        if register == 0xFF43
            && ly != 0
            && self.pixel_x == 0
            && self.fetch_stage() == FetchStage::Tile
            && self.fetch_dot == 0
        {
            self.fine_scroll_x = value & 7;
        }
        let pixel_x = match register {
            0xFF40 if (old_value ^ value) & 0x10 != 0 => self.fetch_pixel_x,
            0xFF40 if (old_value ^ value) & 0x40 != 0 => self.fetch_pixel_x,
            0xFF42 => self.fetch_pixel_x,
            0xFF43 => self.fetch_pixel_x,
            0xFF40 if (old_value ^ value) & 0x08 != 0 => self.fetch_pixel_x,
            _ => self.pixel_x,
        };
        pixel_x.min(159)
    }

    pub(super) fn write_register(&mut self, register: u16, old_value: u8, value: u8) {
        match register {
            0xFF40 if old_value & 0x20 != 0 && value & 0x20 == 0 && self.window_active => {
                self.window_disable_pending = true;
            }
            0xFF4B if self.window_seen && value.saturating_sub(7) > self.pixel_x => {
                self.window_can_retrigger = true;
            }
            _ => {}
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetcher_advances_through_two_dot_stages() {
        let mut timing = Mode3Timing::new(true, 0, Vec::new());
        let expected = [
            FetchStage::Tile,
            FetchStage::Tile,
            FetchStage::DataLow,
            FetchStage::DataLow,
            FetchStage::DataHigh,
            FetchStage::DataHigh,
            FetchStage::Push,
            FetchStage::Push,
        ];

        for stage in expected {
            assert_eq!(timing.fetch_stage(), stage);
            timing.advance_fetcher();
        }

        assert_eq!(timing.fetch_stage(), FetchStage::Tile);
    }
}
