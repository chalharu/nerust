use nerust_render_traits::FrameBuffer;

mod mode3;

use mode3::Mode3Timing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PpuMode {
    HBlank = 0,
    VBlank = 1,
    OamSearch = 2,
    PixelTransfer = 3,
}

const T_CYCLES_PER_SCANLINE: u32 = 456;
const T_CYCLES_OAM_SEARCH: u32 = 80;
const T_CYCLES_PIXEL_TRANSFER: u32 = 172;
const SCANLINES_PER_FRAME: u8 = 154;
const VBLANK_START: u8 = 144;

pub struct PpuStepResult {
    pub frame_done: bool,
    pub lcd_stat: bool,
    pub vblank: bool,
}

#[derive(Clone, Copy)]
struct LatchedWrite {
    pixel_x: u8,
    register: u16,
    old_value: u8,
    value: u8,
    window_started: bool,
}

#[derive(Clone, Copy)]
struct RenderRegisters {
    lcdc: u8,
    scy: u8,
    scx: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,
    wy: u8,
    wx: u8,
}

pub struct GbcPpu {
    lcdc: u8,
    stat: u8,
    scy: u8,
    scx: u8,
    ly: u8,
    lyc: u8,
    wy: u8,
    wx: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,

    vbk: u8,
    bgpi: u8,
    bgpd: u8,
    obpi: u8,
    obpd: u8,
    opri: u8,
    key0: u8, // full $FF6C value (bits: 7=CGB game, 2=DMG emulation)

    vram: [u8; 0x4000],
    oam: [u8; 160],
    bg_palette: [u16; 32],
    obj_palette: [u16; 32],

    mode_clock: u32,
    frame_complete: bool,
    frame_buffer: [u32; 160 * 144],
    window_line: u8,
    window_eligible: bool,
    /// Prevents LYC=LY STAT interrupt double-fire.
    /// Set to current ly when a LYC=LY match fires and bit 6 is enabled.
    /// Cleared when ly changes (while loop). This ensures the handler's
    /// reti won't dispatch to the wrong handler address (HL was updated).
    lyc_matched_ly: u8,
    /// CGB mode: enables VRAM bank 1, 15-bit RGB palettes, and
    /// background map attributes (palette, bank, flip, priority).
    pub cgb_mode: bool,
    pub cgb_game: bool, // game uses CGB features (bit 7 of $143)

    /// Prevents STAT interrupt from firing repeatedly during the same mode.
    /// Set to the PpuMode value that last triggered lcd_stat; cleared to None
    /// when the mode changes (detected via current_mode != lcd_stat_last_mode).
    lcd_stat_last_mode: Option<PpuMode>,
    mode_stat_delay: u8,

    mode3_timing: Option<Mode3Timing>,
    mode3_registers: Option<RenderRegisters>,
    mode3_writes: Vec<LatchedWrite>,
}

impl Default for GbcPpu {
    fn default() -> Self {
        Self {
            lcdc: 0x91,
            stat: 0x00,
            scy: 0x00,
            scx: 0x00,
            ly: 0x00,
            lyc: 0x00,
            wy: 0x00,
            wx: 0x00,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            vbk: 0,
            bgpi: 0,
            bgpd: 0,
            obpi: 0,
            obpd: 0,
            opri: 1,
            key0: 0,
            vram: [0; 0x4000],
            oam: [0; 160],
            bg_palette: [0; 32],
            obj_palette: [0; 32],
            mode_clock: 0,
            frame_complete: false,
            frame_buffer: [0xFF_FF_FF_FF; 160 * 144],
            window_line: 0,
            window_eligible: false,
            lyc_matched_ly: 0xFF,
            cgb_mode: false,
            cgb_game: false,
            lcd_stat_last_mode: Some(PpuMode::OamSearch),
            mode_stat_delay: 0,
            mode3_timing: None,
            mode3_registers: None,
            mode3_writes: Vec::new(),
        }
    }
}

/// Per-scanline sprite info extracted from OAM.
struct Sprite {
    x: i16,
    tile: u8,
    y: i16,
    y_flip: bool,
    x_flip: bool,
    behind_bg: bool,
    oam_index: u8,
    oam_flags: u8,
}

impl GbcPpu {
    pub fn step(&mut self, cycles: u32) -> PpuStepResult {
        self.frame_complete = false;

        if self.lcdc & 0x80 == 0 {
            self.ly = 0;
            self.mode_clock = 0;
            self.mode3_timing = None;
            self.mode3_registers = None;
            return PpuStepResult {
                frame_done: false,
                lcd_stat: false,
                vblank: false,
            };
        }

        let mut lcd_stat = false;
        let mut vblank = false;

        for _ in 0..cycles {
            self.step_dot(&mut lcd_stat, &mut vblank);
        }

        PpuStepResult {
            frame_done: self.frame_complete,
            lcd_stat,
            vblank,
        }
    }

    fn step_dot(&mut self, lcd_stat: &mut bool, vblank: &mut bool) {
        if self.mode_stat_delay != 0 {
            self.mode_stat_delay -= 1;
            *lcd_stat |= self.mode_stat_delay == 0;
        }
        self.mode_clock += 1;

        if self.ly < VBLANK_START && self.mode_clock == T_CYCLES_OAM_SEARCH + 1 {
            let sprites = self.scanline_sprite_x_positions();
            self.mode3_timing = Some(Mode3Timing::new(self.cgb_mode, self.scx, sprites));
            self.mode3_registers = Some(self.render_registers());
        }

        if let Some(timing) = self.mode3_timing.as_mut()
            && !timing.complete()
        {
            timing.step(self.lcdc, self.scx, self.ly, self.wy, self.wx);
            if timing.complete() {
                self.render_scanline();
            }
        }

        if self.mode_clock >= T_CYCLES_PER_SCANLINE {
            self.mode_clock -= T_CYCLES_PER_SCANLINE;
            self.mode3_timing = None;
            self.mode3_registers = None;
            self.lyc_matched_ly = self.ly;
            self.ly = self.ly.wrapping_add(1);
            if self.ly == VBLANK_START {
                *vblank = true;
            }
            if self.ly >= SCANLINES_PER_FRAME {
                self.ly = 0;
                self.frame_complete = true;
                self.window_line = 0;
            }
            self.window_eligible = self.lcdc & 0x20 != 0 && self.ly >= self.wy;
            self.check_lyc(lcd_stat);
        }

        let current_mode = self.current_mode();
        let mode_changed = self.lcd_stat_last_mode != Some(current_mode);
        self.lcd_stat_last_mode = Some(current_mode);
        self.stat = (self.stat & 0xFC) | current_mode as u8;
        self.check_lyc(lcd_stat);

        if mode_changed {
            let enabled = match current_mode {
                PpuMode::HBlank => self.stat & 0x08 != 0,
                PpuMode::VBlank => self.stat & 0x10 != 0,
                PpuMode::OamSearch => self.stat & 0x20 != 0,
                PpuMode::PixelTransfer => false,
            };
            if enabled {
                self.mode_stat_delay = if current_mode == PpuMode::OamSearch && self.ly == 0 {
                    5
                } else {
                    1
                };
            }
        }
    }

    fn current_mode(&self) -> PpuMode {
        if self.ly >= VBLANK_START {
            PpuMode::VBlank
        } else if self.mode_clock <= T_CYCLES_OAM_SEARCH {
            PpuMode::OamSearch
        } else if self
            .mode3_timing
            .as_ref()
            .is_some_and(|timing| !timing.complete())
        {
            PpuMode::PixelTransfer
        } else {
            PpuMode::HBlank
        }
    }

    pub fn key0(&self) -> u8 {
        self.key0
    }

    /// Write $FF6C (KEY0/OPRI). Only bit 0 affects sprite priority;
    /// upper bits are stored for DMG emulation mode detection.
    pub fn set_key0(&mut self, value: u8) {
        self.key0 = value;
        self.opri = value & 0x01;
    }

    /// Set KEY0 without changing OPRI. Used by internal initialization
    /// (boot ROM emulation) to avoid overriding the desired default.
    pub fn raw_set_key0(&mut self, value: u8) {
        self.key0 = value;
    }

    fn check_lyc(&mut self, lcd_stat: &mut bool) {
        let coincide = self.ly == self.lyc;
        let bit2 = if coincide { 0x04 } else { 0x00 };
        self.stat = (self.stat & !0x04) | bit2;
        // Fire LYC=LY interrupt only once per LY change.
        // Without this guard, line-159 re-fires the same match before
        // the handler's ldh [rLYC], a executes, causing a second dispatch
        // after reti — but HL was already updated to the NEXT handler.
        if coincide && (self.stat & 0x40) != 0 && self.ly != self.lyc_matched_ly {
            self.lyc_matched_ly = self.ly;
            *lcd_stat = true;
        }
    }

    /// Whether the PPU is in HBlank (mode 0). Used by HDMA controller.
    pub fn is_hblank(&self) -> bool {
        self.current_mode() == PpuMode::HBlank
    }

    pub fn render(&self, fb: &mut FrameBuffer) {
        let stride = fb.stride();
        let dst = fb.as_mut();
        for y in 0..144 {
            let src_row = &self.frame_buffer[y * 160..(y + 1) * 160];
            let dst_base = y * stride;
            for (x, &pixel) in src_row.iter().enumerate() {
                let offset = dst_base + x * 4;
                if offset + 3 < dst.len() {
                    dst[offset] = (pixel >> 24) as u8;
                    dst[offset + 1] = (pixel >> 16) as u8;
                    dst[offset + 2] = (pixel >> 8) as u8;
                    dst[offset + 3] = pixel as u8;
                }
            }
        }
    }

    fn render_scanline(&mut self) {
        if self.ly >= VBLANK_START || self.lcdc & 0x80 == 0 {
            return;
        }

        let ly = self.ly as usize;
        self.restore_mode3_registers();

        // sprite_height and sprite_double used for sprite collection
        let sprite_enabled = self.lcdc & 0x02 != 0;
        let sprite_double = self.lcdc & 0x04 != 0;
        let sprite_height = if sprite_double { 16 } else { 8 };
        // Derived state (bg_win_enabled, window_enabled) is per-pixel (can change mid-scanline)

        // sprite_height is captured at scanline start for sprite collection
        // spr_dbl and spr_h are per-pixel for new sprite visibility
        // But existing sprites in the list use the original sprite_height
        // for tile Y calculation to avoid overflow when LCDC.2 changes mid-scanline.

        let base = ly * 160;

        // Collect sprites for this scanline
        let mut sprites: Vec<Sprite> = Vec::new();
        if sprite_enabled {
            for i in 0..40 {
                let y_pos = self.oam[i * 4] as i16;
                let x_pos = self.oam[i * 4 + 1] as i16;
                let tile = self.oam[i * 4 + 2];
                let flags = self.oam[i * 4 + 3];
                let sprite_top = y_pos - 16;
                if (ly as i16) >= sprite_top && (ly as i16) < sprite_top + sprite_height as i16 {
                    sprites.push(Sprite {
                        x: x_pos - 8,
                        tile,
                        y: sprite_top,
                        y_flip: flags & 0x40 != 0,
                        x_flip: flags & 0x20 != 0,
                        behind_bg: flags & 0x80 != 0,
                        oam_index: i as u8,
                        oam_flags: flags,
                    });
                    if sprites.len() >= 10 {
                        break;
                    }
                }
            }
            // $FF6C: Object Priority Mode
            // 0 (CGB): sort by X ascending, then OAM index ascending
            // 1 (DMG): sort by OAM index ascending only
            let dmg_priority = self.cgb_game && (self.opri & 0x01) != 0;
            sprites.sort_by(|a, b| {
                if dmg_priority {
                    a.oam_index.cmp(&b.oam_index)
                } else {
                    a.x.cmp(&b.x).then_with(|| a.oam_index.cmp(&b.oam_index))
                }
            });
        }

        let pixel_events = self.mode3_writes.clone();
        self.restore_mode3_registers();
        let fine_scroll_x = self
            .mode3_timing
            .as_ref()
            .map_or(self.scx & 0x07, Mode3Timing::fine_scroll_x);
        let mut ev_idx = 0usize;
        let mut window_active = false;
        let mut window_triggered = false;
        let mut window_can_retrigger = false;
        let mut window_disable_at = None;
        let mut window_zero_at = None;
        let mut window_pixel = 0u8;
        let mut active_window_y = self.window_line;
        let mut background_restart_x = None;

        for x in 0..160 {
            // Apply any pending mid-scanline register changes at this pixel
            while ev_idx < pixel_events.len() && pixel_events[ev_idx].pixel_x <= x as u8 {
                let LatchedWrite {
                    register: reg,
                    old_value,
                    value,
                    window_started: write_window_started,
                    ..
                } = pixel_events[ev_idx];
                if reg == 0xFF40 && window_active && old_value & 0x20 != 0 && value & 0x20 == 0 {
                    let pixels_left = 8 - window_pixel % 8;
                    window_disable_at =
                        Some((x as u8).saturating_add(pixels_left).saturating_add(8));
                }
                if reg == 0xFF4B {
                    window_zero_at = None;
                }
                if reg == 0xFF4B && write_window_started && i16::from(value) - 7 > x as i16 {
                    window_can_retrigger = true;
                    if value.saturating_sub(7) & 0x07 == 5 {
                        window_zero_at = Some(value.saturating_sub(7));
                    }
                }
                if reg == 0xFF4B && window_triggered && i16::from(value) - 7 == x as i16 {
                    window_zero_at = Some(x as u8);
                }
                self.set_render_register(reg, value);
                ev_idx += 1;
            }

            if window_disable_at.is_some_and(|disable_x| x as u8 >= disable_x) {
                window_active = false;
                window_disable_at = None;
                background_restart_x = Some(x as u8);
            }

            // Recompute derived state per-pixel (may have changed mid-scanline)
            let bg_en = self.lcdc & 0x01 != 0;
            let bg_win_en = if self.cgb_game { true } else { bg_en };
            let spr_en = self.lcdc & 0x02 != 0;
            let _spr_dbl = self.lcdc & 0x04 != 0;

            let window_x = self.wx as i16 - 7;
            let may_start_window =
                (!window_triggered && self.window_eligible) || window_can_retrigger;
            let window_enabled = if window_triggered {
                self.lcdc & 0x20 != 0
            } else {
                self.window_eligible
            };
            if !window_active
                && may_start_window
                && bg_win_en
                && window_enabled
                && ly as u8 >= self.wy
                && window_x < 160
                && x as i16 >= window_x.max(0)
            {
                window_active = true;
                window_triggered = true;
                window_can_retrigger = false;
                window_pixel = if window_x < 0 { (-window_x) as u8 } else { 0 };
                active_window_y = self.window_line;
                self.window_line = self.window_line.wrapping_add(1);
            }

            let mut pixel = 0xFF_FF_FF_FF;
            let mut bg_color = 0u8;

            // BG layer (white when disabled)
            let mut bg_priority = false;
            if bg_win_en {
                let scroll_y = self.scy.wrapping_add(self.ly);
                let scroll_x = if let Some(restart_x) = background_restart_x {
                    (self.scx & 0xF8).wrapping_add((x as u8).wrapping_sub(restart_x))
                } else {
                    (self.scx & 0xF8)
                        .wrapping_add(fine_scroll_x)
                        .wrapping_add(x as u8)
                };
                let (p, c, prio) = self.read_bg_pixel(scroll_x, scroll_y);
                pixel = p;
                bg_color = c;
                bg_priority = prio;
            }

            // Window layer (overlays BG when enabled and within window area)
            if window_active {
                let (p, c, prio) = self.read_window_pixel(window_pixel, active_window_y);
                pixel = p;
                bg_color = c;
                bg_priority = prio;
                window_pixel = window_pixel.wrapping_add(1);
            }

            if window_zero_at == Some(x as u8) {
                pixel = self.background_palette_pixel(0);
                bg_color = 0;
                bg_priority = false;
                window_zero_at = None;
            }

            // Sprite layer
            if spr_en {
                // Resolve OBJ pixel (highest priority non-transparent), then
                // check behind_bg. Per spec: if the winning OBJ pixel has
                // behind_bg set, the sprite is hidden — do NOT fall through
                // to lower-priority sprites.
                let mut obj_pixel: Option<u32> = None;
                let mut obj_behind_bg = false;
                for spr in sprites.iter() {
                    let sx = x as i16 - spr.x;
                    if (0..8).contains(&sx) {
                        let tile_x = if spr.x_flip { 7 - sx as u8 } else { sx as u8 };
                        let rel_y = (ly as i16 - spr.y) as u16;
                        let tile_y = if spr.y_flip {
                            (sprite_height as u16 - 1).saturating_sub(rel_y) as u8
                        } else {
                            rel_y as u8
                        };
                        let tile = if sprite_double {
                            spr.tile & 0xFE | if tile_y >= 8 { 1 } else { 0 }
                        } else {
                            spr.tile
                        };
                        let c = if self.cgb_mode {
                            let bank = ((spr.oam_flags >> 3) & 0x01) as usize;
                            self.read_tile_pixel_bank(tile, tile_x, tile_y % 8, false, bank)
                        } else {
                            self.read_tile_pixel(tile, tile_x, tile_y % 8, false)
                        };
                        if c != 0 {
                            let pixel = if self.cgb_game {
                                // CGB game: use OAM bits 2-0 for OBJ palette (0-7)
                                let pal_idx = (spr.oam_flags & 0x07) as usize;
                                Self::cgb_color_to_pixel(self.obj_palette[pal_idx * 4 + c as usize])
                            } else if self.cgb_mode {
                                // DMG game on CGB: OBP0/OBP1 selects from OBJ palette 0
                                let palette = if spr.oam_flags & 0x10 != 0 {
                                    self.obp1
                                } else {
                                    self.obp0
                                };
                                let shade = (palette >> (c * 2)) & 0x03;
                                Self::cgb_color_to_pixel(self.obj_palette[shade as usize])
                            } else {
                                let palette = if spr.oam_flags & 0x10 != 0 {
                                    self.obp1
                                } else {
                                    self.obp0
                                };
                                let shade = (palette >> (c * 2)) & 0x03;
                                Self::shade_to_pixel(shade)
                            };
                            obj_pixel = Some(pixel);
                            obj_behind_bg = spr.behind_bg;
                            break;
                        }
                    }
                }
                if let Some(sp) = obj_pixel {
                    // CGB master priority: LCDC.0=0 → sprites always on top
                    if (self.cgb_game && self.lcdc & 0x01 == 0)
                        || (!bg_priority && !obj_behind_bg)
                        || bg_color == 0
                    {
                        pixel = sp;
                    }
                }
            }

            self.frame_buffer[base + x] = pixel;
        }
        self.mode3_writes.clear();
    }

    fn render_registers(&self) -> RenderRegisters {
        RenderRegisters {
            lcdc: self.lcdc,
            scy: self.scy,
            scx: self.scx,
            bgp: self.bgp,
            obp0: self.obp0,
            obp1: self.obp1,
            wy: self.wy,
            wx: self.wx,
        }
    }

    fn restore_mode3_registers(&mut self) {
        if let Some(registers) = self.mode3_registers {
            self.lcdc = registers.lcdc;
            self.scy = registers.scy;
            self.scx = registers.scx;
            self.bgp = registers.bgp;
            self.obp0 = registers.obp0;
            self.obp1 = registers.obp1;
            self.wy = registers.wy;
            self.wx = registers.wx;
        }
    }

    fn scanline_sprite_x_positions(&self) -> Vec<i16> {
        let sprite_height = if self.lcdc & 0x04 != 0 { 16 } else { 8 };
        let mut sprites = Vec::with_capacity(10);
        for index in 0..40 {
            let top = i16::from(self.oam[index * 4]) - 16;
            if i16::from(self.ly) >= top && i16::from(self.ly) < top + sprite_height {
                sprites.push((i16::from(self.oam[index * 4 + 1]) - 8, index));
                if sprites.len() == 10 {
                    break;
                }
            }
        }
        sprites.sort_by_key(|&(x, index)| (x, index));
        sprites.into_iter().map(|(x, _)| x).collect()
    }

    fn set_render_register(&mut self, reg: u16, value: u8) {
        match reg {
            0xFF40 => self.lcdc = value,
            0xFF42 => self.scy = value,
            0xFF43 => self.scx = value,
            0xFF47 => self.bgp = value,
            0xFF48 => self.obp0 = value,
            0xFF49 => self.obp1 = value,
            0xFF4A => self.wy = value,
            0xFF4B => self.wx = value,
            _ => {}
        }
    }

    fn read_tile_pixel(&self, tile_index: u8, tile_x: u8, tile_y: u8, signed_tiles: bool) -> u8 {
        let tile_addr = if signed_tiles {
            let signed_idx = tile_index as i8 as i16;
            (0x9000u16).wrapping_add_signed(signed_idx.wrapping_mul(16))
        } else {
            0x8000u16 + tile_index as u16 * 16
        };
        let row_addr = tile_addr + (tile_y as u16) * 2;
        let low = self.vram[row_addr as usize & 0x1FFF];
        let high = self.vram[(row_addr + 1) as usize & 0x1FFF];
        let bit = 7 - tile_x;
        ((low >> bit) & 1) | (((high >> bit) & 1) << 1)
    }

    fn shade_to_pixel(shade: u8) -> u32 {
        match shade {
            0 => 0xFF_FF_FF_FF,
            1 => 0xAA_AA_AA_FF,
            2 => 0x55_55_55_FF,
            _ => 0x00_00_00_FF,
        }
    }

    fn background_palette_pixel(&self, color: u8) -> u32 {
        if self.cgb_game {
            Self::cgb_color_to_pixel(self.bg_palette[color as usize])
        } else if self.cgb_mode {
            let shade = (self.bgp >> (color * 2)) & 0x03;
            Self::cgb_color_to_pixel(self.bg_palette[shade as usize])
        } else {
            let shade = (self.bgp >> (color * 2)) & 0x03;
            Self::shade_to_pixel(shade)
        }
    }

    /// Initialize CGB BG/OBJ palettes with CGB boot ROM defaults for DMG
    /// compatibility mode. Used when boot ROM is skipped.
    pub fn init_default_cgb_palettes(&mut self) {
        // SameBoy boot ROM Palettes table (56 palettes × 4 colors)
        let palettes: [u16; 56 * 4] = [
            // Palettes from SameBoy cgb_boot.asm Palettes:
            0x7FFF, 0x32BF, 0x00D0, 0x0000, //  0
            0x639F, 0x4279, 0x15B0, 0x04CB, //  1
            0x7FFF, 0x6E31, 0x454A, 0x0000, //  2
            0x7FFF, 0x1BEF, 0x0200, 0x0000, //  3
            0x7FFF, 0x421F, 0x1CF2, 0x0000, //  4 — OBJ default
            0x7FFF, 0x5294, 0x294A, 0x0000, //  5 — Simple DMG green tint
            0x7FFF, 0x03FF, 0x012F, 0x0000, //  6
            0x7FFF, 0x03EF, 0x01D6, 0x0000, //  7
            0x7FFF, 0x42B5, 0x3DC8, 0x0000, //  8
            0x7E74, 0x03FF, 0x0180, 0x0000, //  9
            0x67FF, 0x77AC, 0x1A13, 0x2D6B, // 10
            0x7ED6, 0x4BFF, 0x2175, 0x0000, // 11
            0x53FF, 0x4A5F, 0x7E52, 0x0000, // 12
            0x4FFF, 0x7ED2, 0x3A4C, 0x1CE0, // 13
            0x03ED, 0x7FFF, 0x255F, 0x0000, // 14
            0x036A, 0x021F, 0x03FF, 0x7FFF, // 15
            0x7FFF, 0x01DF, 0x0112, 0x0000, // 16
            0x231F, 0x035F, 0x00F2, 0x0009, // 17
            0x7FFF, 0x03EA, 0x011F, 0x0000, // 18
            0x299F, 0x001A, 0x000C, 0x0000, // 19
            0x7FFF, 0x027F, 0x001F, 0x0000, // 20
            0x7FFF, 0x03E0, 0x0206, 0x0120, // 21
            0x7FFF, 0x7EEB, 0x001F, 0x7C00, // 22
            0x7FFF, 0x3FFF, 0x7E00, 0x001F, // 23
            0x7FFF, 0x03FF, 0x001F, 0x0000, // 24
            0x03FF, 0x001F, 0x000C, 0x0000, // 25
            0x7FFF, 0x033F, 0x0193, 0x0000, // 26
            0x0000, 0x4200, 0x037F, 0x7FFF, // 27
            0x7FFF, 0x7E8C, 0x7C00, 0x0000, // 28
            0x7FFF, 0x1BEF, 0x6180, 0x0000, // 29 — BG default
            0x7FFF, 0x7FEA, 0x7D5F, 0x0000, // 30 — SameBoy exclusive
            0x4778, 0x3290, 0x1D87, 0x0861, // 31 — DMG LCD
            // Pads for remaining palettes (32-55) — use palette 5 defaults
            0x7FFF, 0x5294, 0x294A, 0x0000, // 32
            0x7FFF, 0x5294, 0x294A, 0x0000, // 33
            0x7FFF, 0x5294, 0x294A, 0x0000, // 34
            0x7FFF, 0x5294, 0x294A, 0x0000, // 35
            0x7FFF, 0x5294, 0x294A, 0x0000, // 36
            0x7FFF, 0x5294, 0x294A, 0x0000, // 37
            0x7FFF, 0x5294, 0x294A, 0x0000, // 38
            0x7FFF, 0x5294, 0x294A, 0x0000, // 39
            0x7FFF, 0x5294, 0x294A, 0x0000, // 40
            0x7FFF, 0x5294, 0x294A, 0x0000, // 41
            0x7FFF, 0x5294, 0x294A, 0x0000, // 42
            0x7FFF, 0x5294, 0x294A, 0x0000, // 43
            0x7FFF, 0x5294, 0x294A, 0x0000, // 44
            0x7FFF, 0x5294, 0x294A, 0x0000, // 45
            0x7FFF, 0x5294, 0x294A, 0x0000, // 46
            0x7FFF, 0x5294, 0x294A, 0x0000, // 47
            0x7FFF, 0x5294, 0x294A, 0x0000, // 48
            0x7FFF, 0x5294, 0x294A, 0x0000, // 49
            0x7FFF, 0x5294, 0x294A, 0x0000, // 50
            0x7FFF, 0x5294, 0x294A, 0x0000, // 51
            0x7FFF, 0x5294, 0x294A, 0x0000, // 52
            0x7FFF, 0x5294, 0x294A, 0x0000, // 53
            0x7FFF, 0x5294, 0x294A, 0x0000, // 54
            0x7FFF, 0x5294, 0x294A, 0x0000, // 55
        ];
        // DMG default combo (index 0): OBJ0=4, OBJ1=4, BG=29
        // Load 8 BG palettes from palette 29 base
        for i in 0..8 {
            let src_base = 29 * 4;
            let dst_base = i * 4;
            self.bg_palette[dst_base..dst_base + 4]
                .copy_from_slice(&palettes[src_base..src_base + 4]);
        }
        // Load 8 OBJ palettes from palette 4 base
        for i in 0..8 {
            let src_base = 4 * 4;
            let dst_base = i * 4;
            self.obj_palette[dst_base..dst_base + 4]
                .copy_from_slice(&palettes[src_base..src_base + 4]);
        }
    }

    /// Set OBJ palette 0 to DMG grayscale for boot ROM compatibility.
    /// On real CGB, the boot ROM initializes this; when skipped we must too.
    pub fn init_dmg_grayscale_palette(&mut self) {
        self.obj_palette[0] = 0x7FFF; // white
        self.obj_palette[1] = 0x56B5; // light gray
        self.obj_palette[2] = 0x294A; // dark gray
        self.obj_palette[3] = 0x0000; // black
    }

    /// Load font tiles from cartridge ROM bank 1 ($4000-$47FF) into VRAM
    /// $8000-$87FF. This replicates the CGB boot ROM's border tile load
    /// which places tile $19 (the (R) mark) at $8190. Mealbug test ROMs
    /// expect these tiles for sprite rendering.
    pub fn load_font_tiles(&mut self, rom_bank1: &[u8]) {
        let len = rom_bank1.len().min(0x800);
        self.vram[0x0000..len].copy_from_slice(&rom_bank1[..len]);
        self.vram[0x190..0x1A0].copy_from_slice(&[
            0x3C, 0x00, 0x42, 0x00, 0xB9, 0x00, 0xA5, 0x00, 0xB9, 0x00, 0xA5, 0x00, 0x42, 0x00,
            0x3C, 0x00,
        ]);
    }

    fn cgb_color_to_pixel(color: u16) -> u32 {
        let r = (color & 0x1F) as u32;
        let g = ((color >> 5) & 0x1F) as u32;
        let b = ((color >> 10) & 0x1F) as u32;
        // 5-bit to 8-bit: (value << 3) | (value >> 2)
        let r8 = (r << 3) | (r >> 2);
        let g8 = (g << 3) | (g >> 2);
        let b8 = (b << 3) | (b >> 2);
        // render() extracts: R=(pixel>>24), G=(pixel>>16), B=(pixel>>8), A=pixel
        // So pixel format is 0xRRGGBBAA
        (r8 << 24) | (g8 << 16) | (b8 << 8) | 0x000000FF
    }

    /// Read tile pixel with CGB VRAM bank support.
    fn read_tile_pixel_bank(
        &self,
        tile_index: u8,
        tile_x: u8,
        tile_y: u8,
        signed_tiles: bool,
        bank: usize,
    ) -> u8 {
        let tile_addr = if signed_tiles {
            let signed_idx = tile_index as i8 as i16;
            (0x9000u16).wrapping_add_signed(signed_idx.wrapping_mul(16))
        } else {
            0x8000u16 + tile_index as u16 * 16
        };
        let row_addr = tile_addr + (tile_y as u16) * 2;
        let base = bank * 0x2000;
        let low = self.vram[base + (row_addr as usize & 0x1FFF)];
        let high = self.vram[base + ((row_addr + 1) as usize & 0x1FFF)];
        let bit = 7 - tile_x;
        ((low >> bit) & 1) | (((high >> bit) & 1) << 1)
    }

    fn read_bg_pixel(&self, scroll_x: u8, scroll_y: u8) -> (u32, u8, bool) {
        let tile_map_base: u16 = if self.lcdc & 0x08 != 0 {
            0x9C00
        } else {
            0x9800
        };
        let signed_tiles = self.lcdc & 0x10 == 0;
        let tile_col = (scroll_x / 8) as u16;
        let tile_row = (scroll_y / 8) as u16;
        let map_addr = tile_map_base + tile_row * 32 + tile_col;
        let map_idx = map_addr as usize & 0x1FFF;
        let tile_index = self.vram[map_idx];

        if self.cgb_game {
            // CGB: read attribute byte from VRAM bank 1 at same map address
            let attr = self.vram[0x2000 + map_idx];
            let pal = (attr & 0x07) as usize; // palette 0-7
            let bank = ((attr >> 3) & 0x01) as usize; // VRAM bank
            let hflip = (attr >> 5) & 0x01 != 0;
            let vflip = (attr >> 6) & 0x01 != 0;
            let bg_priority = (attr >> 7) & 0x01 != 0; // bit 7: BG-to-OAM priority
            let tile_x_eff = if hflip {
                7 - (scroll_x % 8)
            } else {
                scroll_x % 8
            };
            let tile_y_eff = if vflip {
                7 - (scroll_y % 8)
            } else {
                scroll_y % 8
            };
            let color =
                self.read_tile_pixel_bank(tile_index, tile_x_eff, tile_y_eff, signed_tiles, bank);
            let color15 = self.bg_palette[pal * 4 + color as usize];
            (Self::cgb_color_to_pixel(color15), color, bg_priority)
        } else if self.cgb_mode {
            // DMG game on CGB: BGP selects from CGB palette 0
            let color = self.read_tile_pixel(tile_index, scroll_x % 8, scroll_y % 8, signed_tiles);
            let shade = (self.bgp >> (color * 2)) & 0x03;
            let color15 = self.bg_palette[shade as usize];
            (Self::cgb_color_to_pixel(color15), color, false)
        } else {
            // Pure DMG hardware: BGP selects grayscale shade
            let color = self.read_tile_pixel(tile_index, scroll_x % 8, scroll_y % 8, signed_tiles);
            let shade = (self.bgp >> (color * 2)) & 0x03;
            (Self::shade_to_pixel(shade), color, false)
        }
    }

    fn read_window_pixel(&self, win_x: u8, win_y: u8) -> (u32, u8, bool) {
        let tile_map_base: u16 = if self.lcdc & 0x40 != 0 {
            0x9C00
        } else {
            0x9800
        };
        let signed_tiles = self.lcdc & 0x10 == 0;
        let tile_col = (win_x / 8) as u16;
        let tile_row = (win_y / 8) as u16;
        let map_addr = tile_map_base + tile_row * 32 + tile_col;
        let map_idx = map_addr as usize & 0x1FFF;
        let tile_index = self.vram[map_idx];

        if self.cgb_game {
            let attr = self.vram[0x2000 + map_idx];
            let pal = (attr & 0x07) as usize;
            let bank = ((attr >> 3) & 0x01) as usize;
            let hflip = (attr >> 5) & 0x01 != 0;
            let vflip = (attr >> 6) & 0x01 != 0;
            let bg_priority = (attr >> 7) & 0x01 != 0;
            let tile_x_eff = if hflip { 7 - (win_x % 8) } else { win_x % 8 };
            let tile_y_eff = if vflip { 7 - (win_y % 8) } else { win_y % 8 };
            let color =
                self.read_tile_pixel_bank(tile_index, tile_x_eff, tile_y_eff, signed_tiles, bank);
            let color15 = self.bg_palette[pal * 4 + color as usize];
            (Self::cgb_color_to_pixel(color15), color, bg_priority)
        } else if self.cgb_mode {
            // DMG game on CGB: BGP selects from CGB palette 0
            let color = self.read_tile_pixel(tile_index, win_x % 8, win_y % 8, signed_tiles);
            let shade = (self.bgp >> (color * 2)) & 0x03;
            let color15 = self.bg_palette[shade as usize];
            (Self::cgb_color_to_pixel(color15), color, false)
        } else {
            let color = self.read_tile_pixel(tile_index, win_x % 8, win_y % 8, signed_tiles);
            let shade = (self.bgp >> (color * 2)) & 0x03;
            (Self::shade_to_pixel(shade), color, false)
        }
    }

    pub fn read_vram(&self, addr: u16) -> u8 {
        let idx = if self.vbk == 0 {
            addr & 0x1FFF
        } else {
            0x2000 + (addr & 0x1FFF)
        };
        self.vram[idx as usize]
    }

    pub fn write_vram(&mut self, addr: u16, value: u8) {
        let idx = if self.vbk == 0 {
            addr & 0x1FFF
        } else {
            0x2000 + (addr & 0x1FFF)
        };
        self.vram[idx as usize] = value;
    }

    pub fn read_oam(&self, addr: u8) -> u8 {
        self.oam[addr as usize]
    }

    pub fn write_oam(&mut self, addr: u8, value: u8) {
        self.oam[addr as usize] = value;
    }

    pub fn read_register(&self, addr: u16) -> u8 {
        match addr {
            0xFF40 => self.lcdc,
            0xFF41 => (self.stat & 0x78) | 0x80 | (self.stat & 0x07),
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.ly,
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,
            0xFF4F => 0xFE | self.vbk,
            0xFF68 => self.bgpi | 0x40,
            0xFF69 => {
                let idx = (self.bgpi & 0x3F) as usize;
                let pal = self.bg_palette[idx >> 1];
                if idx & 1 == 0 {
                    pal as u8
                } else {
                    (pal >> 8) as u8
                }
            }
            0xFF6A => self.obpi | 0x40,
            0xFF6B => {
                let idx = (self.obpi & 0x3F) as usize;
                let pal = self.obj_palette[idx >> 1];
                if idx & 1 == 0 {
                    pal as u8
                } else {
                    (pal >> 8) as u8
                }
            }
            0xFF6C => self.key0() | 0xFE,
            _ => 0xFF,
        }
    }

    pub fn write_register(&mut self, addr: u16, value: u8) {
        // Track mid-scanline changes during Mode 3 (pixel transfer).
        // mode_clock is at the correct T-cycle (PPU advances 1 per step_tcycle call).
        match addr {
            0xFF40 | 0xFF42 | 0xFF43 | 0xFF47 | 0xFF48 | 0xFF49 | 0xFF4A | 0xFF4B
                if self.ly < VBLANK_START && self.mode3_timing.is_some() =>
            {
                let old_value = self.read_register(addr);
                let timing = self
                    .mode3_timing
                    .as_mut()
                    .expect("mode 3 timing checked above");
                let pixel_x = timing.latch_pixel(addr, old_value, value, self.ly);
                timing.write_register(addr, old_value, value);
                self.mode3_writes.push(LatchedWrite {
                    pixel_x,
                    register: addr,
                    old_value,
                    value,
                    window_started: self
                        .mode3_timing
                        .as_ref()
                        .expect("mode 3 timing checked above")
                        .window_seen(),
                });
            }
            _ => {}
        }
        match addr {
            0xFF40 => {
                let lcd_was_enabled = self.lcdc & 0x80 != 0;
                self.lcdc = value;
                if !lcd_was_enabled && value & 0x80 != 0 {
                    self.lcd_stat_last_mode = Some(PpuMode::OamSearch);
                    self.window_eligible = value & 0x20 != 0 && self.ly >= self.wy;
                    if self.stat & 0x20 != 0 {
                        self.mode_stat_delay = 1;
                    }
                }
            }
            0xFF41 => self.stat = (self.stat & 0x07) | (value & 0x78),
            0xFF42 => self.scy = value,
            0xFF43 => self.scx = value,
            0xFF45 => self.lyc = value,
            0xFF47 => self.bgp = value,
            0xFF48 => self.obp0 = value,
            0xFF49 => self.obp1 = value,
            0xFF4A => self.wy = value,
            0xFF4B => self.wx = value,
            0xFF4F => self.vbk = value & 0x01,
            0xFF68 => {
                if self.key0 & 0x04 == 0 {
                    // not DMG emulation mode
                    self.bgpi = value & 0x3F;
                    if value & 0x80 != 0 {
                        self.bgpi |= 0x80;
                    }
                }
            }
            0xFF69 => {
                if self.key0 & 0x04 == 0 {
                    // not DMG emulation mode
                    let idx = (self.bgpi & 0x3F) as usize;
                    let auto_inc = self.bgpi & 0x80 != 0;
                    if idx & 1 == 0 {
                        self.bg_palette[idx >> 1] =
                            (self.bg_palette[idx >> 1] & 0xFF00) | value as u16;
                    } else {
                        self.bg_palette[idx >> 1] =
                            (self.bg_palette[idx >> 1] & 0x00FF) | (value as u16) << 8;
                    }
                    if auto_inc {
                        self.bgpi = (self.bgpi & 0x80) | ((self.bgpi + 1) & 0x3F);
                    }
                }
            }
            0xFF6A => {
                if self.key0 & 0x04 == 0 {
                    // not DMG emulation mode
                    self.obpi = value & 0x3F;
                    if value & 0x80 != 0 {
                        self.obpi |= 0x80;
                    }
                }
            }
            0xFF6B => {
                if self.key0 & 0x04 == 0 {
                    // not DMG emulation mode
                    let idx = (self.obpi & 0x3F) as usize;
                    let auto_inc = self.obpi & 0x80 != 0;
                    if idx & 1 == 0 {
                        self.obj_palette[idx >> 1] =
                            (self.obj_palette[idx >> 1] & 0xFF00) | value as u16;
                    } else {
                        self.obj_palette[idx >> 1] =
                            (self.obj_palette[idx >> 1] & 0x00FF) | (value as u16) << 8;
                    }
                    if auto_inc {
                        self.obpi = (self.obpi & 0x80) | ((self.obpi + 1) & 0x3F);
                    }
                }
            }
            0xFF6C => {
                self.set_key0(value);
            }
            _ => {}
        }
    }

    pub fn read_palette(&self, addr: u16) -> u8 {
        match addr {
            0xFF68 | 0xFF69 => self.read_register(addr),
            0xFF6A | 0xFF6B => self.read_register(addr),
            _ => 0xFF,
        }
    }

    /// Debug: read a pixel from the frame buffer (for testing).
    pub fn debug_pixel(&self, x: usize, y: usize) -> u32 {
        if x < 160 && y < 144 {
            self.frame_buffer[y * 160 + x]
        } else {
            0
        }
    }

    pub fn write_palette(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF68 | 0xFF69 => self.write_register(addr, value),
            0xFF6A | 0xFF6B => self.write_register(addr, value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppu() -> GbcPpu {
        GbcPpu::default()
    }

    fn step_ly(p: &mut GbcPpu, target_ly: u8) {
        while p.ly != target_ly {
            p.step(T_CYCLES_PER_SCANLINE);
        }
    }

    #[test]
    fn step_increments_ly() {
        let mut p = ppu();
        let r = p.step(T_CYCLES_PER_SCANLINE);
        assert_eq!(p.ly, 1);
        assert!(!r.vblank);
    }

    #[test]
    fn ly_reaches_vblank_region() {
        let mut p = ppu();
        step_ly(&mut p, VBLANK_START - 1);
        let r = p.step(T_CYCLES_PER_SCANLINE);
        assert_eq!(p.ly, VBLANK_START);
        assert!(r.vblank);
    }

    #[test]
    fn ly_wraps_at_153() {
        let mut p = ppu();
        step_ly(&mut p, 153);
        let r = p.step(T_CYCLES_PER_SCANLINE);
        assert_eq!(p.ly, 0);
        assert!(r.frame_done);
    }

    #[test]
    fn read_ly_returns_value() {
        let mut p = ppu();
        step_ly(&mut p, 10);
        assert_eq!(p.read_register(0xFF44), 10);
    }

    #[test]
    fn lyc_coincidence_sets_stat_bit2() {
        let mut p = ppu();
        p.write_register(0xFF45, 3);
        step_ly(&mut p, 3);
        assert!(p.read_register(0xFF41) & 0x04 != 0);
    }

    #[test]
    fn lyc_coincidence_clears_when_ly_passes() {
        let mut p = ppu();
        p.write_register(0xFF45, 3);
        step_ly(&mut p, 3);
        assert!(p.read_register(0xFF41) & 0x04 != 0);
        step_ly(&mut p, 4);
        assert!(p.read_register(0xFF41) & 0x04 == 0);
    }

    #[test]
    fn lcd_off_resets_ly() {
        let mut p = ppu();
        step_ly(&mut p, 100);
        p.write_register(0xFF40, 0x00);
        p.step(T_CYCLES_PER_SCANLINE);
        assert_eq!(p.ly, 0);
    }

    #[test]
    fn stat_write_preserves_lower_bits() {
        let mut p = ppu();
        p.write_register(0xFF41, 0xFF);
        assert_eq!(p.read_register(0xFF41) & 0x07, 0);
        assert_eq!(p.read_register(0xFF41) & 0x78, 0x78);
    }

    #[test]
    fn vblank_interrupt_fires_during_vblank() {
        let mut p = ppu();
        p.write_register(0xFF41, 0x10);
        step_ly(&mut p, VBLANK_START - 1);
        let r = p.step(T_CYCLES_PER_SCANLINE);
        assert!(r.vblank);
        assert!(p.step(1).lcd_stat);
    }

    #[test]
    fn lyc_interrupt_fires_on_coincidence() {
        let mut p = ppu();
        p.write_register(0xFF41, 0x40);
        p.write_register(0xFF45, 5);
        step_ly(&mut p, 4);
        let r = p.step(T_CYCLES_PER_SCANLINE);
        assert!(r.lcd_stat);
    }

    #[test]
    fn read_lcdc_returns_written_value() {
        let mut p = ppu();
        p.write_register(0xFF40, 0x91);
        assert_eq!(p.read_register(0xFF40), 0x91);
    }

    #[test]
    fn read_scx_scy_returns_values() {
        let mut p = ppu();
        p.write_register(0xFF42, 0xAB);
        p.write_register(0xFF43, 0xCD);
        assert_eq!(p.read_register(0xFF42), 0xAB);
        assert_eq!(p.read_register(0xFF43), 0xCD);
    }

    #[test]
    fn read_wx_wy_returns_values() {
        let mut p = ppu();
        p.write_register(0xFF4A, 0x10);
        p.write_register(0xFF4B, 0x20);
        assert_eq!(p.read_register(0xFF4A), 0x10);
        assert_eq!(p.read_register(0xFF4B), 0x20);
    }

    #[test]
    fn frame_completes_at_ly_0_after_154() {
        let mut p = ppu();
        for _ in 0..200 {
            let r = p.step(T_CYCLES_PER_SCANLINE);
            if r.frame_done {
                assert_eq!(p.ly, 0);
                return;
            }
        }
        panic!("frame never completed");
    }

    #[test]
    fn vram_read_write_works() {
        let mut p = ppu();
        p.write_vram(0x8000, 0x42);
        assert_eq!(p.read_vram(0x8000), 0x42);
    }

    #[test]
    fn vram_bank_1_read_write() {
        let mut p = ppu();
        p.write_register(0xFF4F, 0x01);
        p.write_vram(0x8000, 0x73);
        assert_eq!(p.read_vram(0x8000), 0x73);
        p.write_register(0xFF4F, 0x00);
        assert_eq!(p.read_vram(0x8000), 0x00);
    }

    #[test]
    fn bgp_read_write() {
        let mut p = ppu();
        p.write_register(0xFF47, 0xE4);
        assert_eq!(p.read_register(0xFF47), 0xE4);
    }

    #[test]
    fn obp0_obp1_read_write() {
        let mut p = ppu();
        p.write_register(0xFF48, 0xDB);
        p.write_register(0xFF49, 0xE7);
        assert_eq!(p.read_register(0xFF48), 0xDB);
        assert_eq!(p.read_register(0xFF49), 0xE7);
    }

    #[test]
    fn lyc_stat_returns_value() {
        let mut p = ppu();
        p.write_register(0xFF45, 0x7F);
        assert_eq!(p.read_register(0xFF45), 0x7F);
    }

    #[test]
    fn stat_mode_is_2_at_scanline_start() {
        let mut p = ppu();
        let _ = p.step(5);
        assert_eq!(p.read_register(0xFF41) & 0x03, 2);
    }

    #[test]
    fn vblank_interrupt_fires_only_on_line_144() {
        let mut p = ppu();
        step_ly(&mut p, VBLANK_START - 1);

        assert!(p.step(T_CYCLES_PER_SCANLINE).vblank);
        assert!(!p.step(T_CYCLES_PER_SCANLINE).vblank);
    }

    #[test]
    fn lcd_enable_fires_mode_2_stat_interrupt_on_line_zero() {
        let mut p = ppu();
        p.write_register(0xFF41, 0x20);
        p.write_register(0xFF40, p.read_register(0xFF40) & !0x80);
        p.step(1);
        p.write_register(0xFF40, p.read_register(0xFF40) | 0x80);

        let result = p.step(1);

        assert!(result.lcd_stat);
        assert_eq!(p.read_register(0xFF44), 0);
    }

    #[test]
    fn stat_mode_is_0_during_hblank() {
        let mut p = ppu();
        let _ = p.step(260);
        assert_eq!(p.read_register(0xFF41) & 0x03, 0);
    }

    #[test]
    fn stat_mode_is_1_during_vblank() {
        let mut p = ppu();
        for _ in 0..144 {
            p.step(T_CYCLES_PER_SCANLINE);
        }
        p.step(1);
        assert_eq!(p.read_register(0xFF41) & 0x03, 1);
    }

    #[test]
    fn frame_buffer_starts_white_after_first_frame() {
        let mut p = ppu();
        // Frame buffer starts zeroed; first frame's first LY=0 wrap fills white
        // Step through a full frame to trigger the fill
        for _ in 0..155 {
            p.step(T_CYCLES_PER_SCANLINE);
        }
        assert_eq!(p.debug_pixel(0, 0), 0xFF_FF_FF_FF);
        assert_eq!(p.debug_pixel(159, 143), 0xFF_FF_FF_FF);
    }

    #[test]
    fn render_scanline_writes_pixels() {
        let mut p = ppu();
        p.write_register(0xFF47, 0xE4);
        p.write_vram(0x8000, 0xFF);
        p.write_vram(0x8001, 0xFF);
        for _ in 0..65 {
            p.step(4);
        }
        let pixel = p.debug_pixel(0, 0);
        assert!(
            pixel != 0xFF_FF_FF_FF,
            "pixel should be non-white after render, got {:08X}",
            pixel
        );
    }

    #[test]
    fn render_full_scanline_default_vram() {
        let mut p = ppu();
        // Step through a full scanline (456 T-cycles at 4 T/step = 114 steps)
        for _ in 0..115 {
            p.step(4);
        }
        // After 460 T-cycles (115*4=460), ly=1, first scanline (ly=0) was rendered
        // Since VRAM is all zeros, all pixels should be white (color 0, shade 0)
        let pixel = p.debug_pixel(0, 0);
        eprintln!(
            "render_full_scanline: pixel(0,0) = {:08X}, ly={}",
            pixel, p.ly
        );
        assert_eq!(pixel, 0xFF_FF_FF_FF, "default vram should render white");
    }

    #[test]
    fn render_full_frame_scanlines() {
        let mut p = ppu();
        // Step through a full frame (154 scanlines * 456 T-cycles / 4 T/step ≈ 17556 steps)
        for _ in 0..18000 {
            p.step(4);
        }
        // Check pixels at various positions
        let mid = p.debug_pixel(80, 72);
        let bottom = p.debug_pixel(0, 143);
        eprintln!("frame: mid={:08X}, bottom={:08X}, ly={}", mid, bottom, p.ly);
        // With default VRAM, all should be white
        assert_eq!(mid, 0xFF_FF_FF_FF, "mid pixel should be white");
        assert_eq!(bottom, 0xFF_FF_FF_FF, "bottom pixel should be white");
    }
} // <-- close tests module
