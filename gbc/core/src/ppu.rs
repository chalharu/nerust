use nerust_render_traits::FrameBuffer;

mod pipeline;

use pipeline::{Mode3Pipeline, Registers, Sprite};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PpuMode {
    HBlank = 0,
    VBlank = 1,
    OamSearch = 2,
    PixelTransfer = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OamBugKind {
    Read,
    Write,
    ReadInc,
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PpuState {
    registers: [u8; 18],
    vram: Vec<u8>,
    oam: Vec<u8>,
    bg_palette: Vec<u16>,
    obj_palette: Vec<u16>,
    mode_clock: u32,
    frame_complete: bool,
    frame_buffer: Vec<u32>,
    window_line: u8,
    window_eligible: bool,
    prev_lyc_coincide: bool,
    cgb_mode: bool,
    cgb_game: bool,
    cgb_revision_d: bool,
    stat_signal: bool,
    stat_forced: bool,
    vblank_if_countdown: u8,
    lcd_on_delay: u32,
    lcd_on_hblank_extra: u32,
    ly_for_comparison: i16,
    lcd_on_short_line: bool,
    mode3_scx_penalty: u32,
    mode3_sprite_penalty: u32,
    wx_written_during_oam: bool,
    accessed_oam_row: u8,
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
    /// Previous LY=LYC coincidence bit. The LYC STAT interrupt is edge
    /// triggered: it only fires on a false->true transition, so re-enabling
    /// the LCD while the comparison already holds (LY=LYC unchanged) does
    /// not raise a second interrupt.
    prev_lyc_coincide: bool,
    /// CGB mode: enables VRAM bank 1, 15-bit RGB palettes, and
    /// background map attributes (palette, bank, flip, priority).
    pub cgb_mode: bool,
    pub cgb_game: bool, // game uses CGB features (bit 7 of $143)
    pub cgb_revision_d: bool,

    /// Current level of the combined STAT interrupt signal. The LCD STAT
    /// interrupt (IF bit 1) is requested on a rising edge of this signal.
    /// The signal is the OR of the enabled STAT sources:
    ///   - bit 3: mode 0 (HBlank)
    ///   - bit 4: mode 1 (VBlank)
    ///   - bit 5: mode 2 (OAM) — also active during VBlank (see below)
    ///   - bit 6: LY=LYC coincidence
    stat_signal: bool,
    /// Forced-high override for the STAT signal, held until the PPU enters
    /// mode 3. Models the mode-2 interrupt pulse right after the LCD is
    /// turned on while STAT bit 5 is set.
    stat_forced: bool,
    /// T-cycles until the VBlank interrupt flag is raised. On real hardware
    /// the VBL IF is set 4 T-cycles after LY becomes 144.
    vblank_if_countdown: u8,

    /// Remaining T-cycles after LCDC.7 0->1 before the PPU starts the first
    /// scanline. During this window STAT mode bits read as 00 and the LYC
    /// comparison clock is not running.
    lcd_on_delay: u32,
    lcd_on_hblank_extra: u32,
    ly_for_comparison: i16,
    /// The first line after the LCD is turned on is shorter than a normal
    /// scanline (448 T-cycles; mode 2 is 76 and mode 0 is 4+8 shorter).
    lcd_on_short_line: bool,

    /// SCX fine-scroll penalty (SCX & 7) latched at the start of mode 3;
    /// extends the pixel transfer period.
    mode3_scx_penalty: u32,
    /// Sprite-fetch stalls accumulated during the current mode 3.
    mode3_sprite_penalty: u32,
    wx_written_during_oam: bool,

    /// DMG OAM bug: OAM row currently being scanned during mode 2 (OAM
    /// search). 0xFF means OAM is not being accessed (CGB, LCD off, or not
    /// in mode 2). A 16-bit CPU operation targeting $FE00-$FEFF during OAM
    /// search corrupts this row by copying from 8 bytes earlier.
    accessed_oam_row: u8,

    mode3_pipeline: Option<Mode3Pipeline>,
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
            key0: 0xFF,
            vram: [0; 0x4000],
            oam: [0; 160],
            bg_palette: [0xFFFF; 32],
            obj_palette: [0xFFFF; 32],
            mode_clock: 0,
            frame_complete: false,
            frame_buffer: [0xFF_FF_FF_FF; 160 * 144],
            window_line: 0,
            window_eligible: false,
            prev_lyc_coincide: false,
            cgb_mode: false,
            cgb_game: false,
            cgb_revision_d: true,
            stat_signal: false,
            stat_forced: false,
            vblank_if_countdown: 0,
            lcd_on_delay: 0,
            lcd_on_hblank_extra: 0,
            ly_for_comparison: 0,
            lcd_on_short_line: false,
            mode3_scx_penalty: 0,
            mode3_sprite_penalty: 0,
            wx_written_during_oam: false,
            accessed_oam_row: 0xFF,
            mode3_pipeline: None,
        }
    }
}

impl GbcPpu {
    pub(crate) fn export_state(&self) -> Result<PpuState, String> {
        if self.mode3_pipeline.is_some() {
            return Err("PPU state can only be saved at a frame boundary".into());
        }
        Ok(PpuState {
            registers: [
                self.lcdc, self.stat, self.scy, self.scx, self.ly, self.lyc, self.wy, self.wx,
                self.bgp, self.obp0, self.obp1, self.vbk, self.bgpi, self.bgpd, self.obpi,
                self.obpd, self.opri, self.key0,
            ],
            vram: self.vram.to_vec(),
            oam: self.oam.to_vec(),
            bg_palette: self.bg_palette.to_vec(),
            obj_palette: self.obj_palette.to_vec(),
            mode_clock: self.mode_clock,
            frame_complete: self.frame_complete,
            frame_buffer: self.frame_buffer.to_vec(),
            window_line: self.window_line,
            window_eligible: self.window_eligible,
            prev_lyc_coincide: self.prev_lyc_coincide,
            cgb_mode: self.cgb_mode,
            cgb_game: self.cgb_game,
            cgb_revision_d: self.cgb_revision_d,
            stat_signal: self.stat_signal,
            stat_forced: self.stat_forced,
            vblank_if_countdown: self.vblank_if_countdown,
            lcd_on_delay: self.lcd_on_delay,
            lcd_on_hblank_extra: self.lcd_on_hblank_extra,
            ly_for_comparison: self.ly_for_comparison,
            lcd_on_short_line: self.lcd_on_short_line,
            mode3_scx_penalty: self.mode3_scx_penalty,
            mode3_sprite_penalty: self.mode3_sprite_penalty,
            wx_written_during_oam: self.wx_written_during_oam,
            accessed_oam_row: self.accessed_oam_row,
        })
    }

    pub(crate) fn import_state(&mut self, state: PpuState) -> Result<(), String> {
        if state.vram.len() != 0x4000
            || state.oam.len() != 160
            || state.bg_palette.len() != 32
            || state.obj_palette.len() != 32
            || state.frame_buffer.len() != 160 * 144
        {
            return Err("PPU state buffer length mismatch".into());
        }
        if state.registers[4] >= SCANLINES_PER_FRAME
            || state.mode_clock >= T_CYCLES_PER_SCANLINE
            || state.registers[11] > 1
        {
            return Err("PPU timing or bank state out of range".into());
        }
        let mut candidate = Self::default();
        [
            candidate.lcdc,
            candidate.stat,
            candidate.scy,
            candidate.scx,
            candidate.ly,
            candidate.lyc,
            candidate.wy,
            candidate.wx,
            candidate.bgp,
            candidate.obp0,
            candidate.obp1,
            candidate.vbk,
            candidate.bgpi,
            candidate.bgpd,
            candidate.obpi,
            candidate.obpd,
            candidate.opri,
            candidate.key0,
        ] = state.registers;
        candidate.vram.copy_from_slice(&state.vram);
        candidate.oam.copy_from_slice(&state.oam);
        candidate.bg_palette.copy_from_slice(&state.bg_palette);
        candidate.obj_palette.copy_from_slice(&state.obj_palette);
        candidate.mode_clock = state.mode_clock;
        candidate.frame_complete = state.frame_complete;
        candidate.frame_buffer.copy_from_slice(&state.frame_buffer);
        candidate.window_line = state.window_line;
        candidate.window_eligible = state.window_eligible;
        candidate.prev_lyc_coincide = state.prev_lyc_coincide;
        candidate.cgb_mode = state.cgb_mode;
        candidate.cgb_game = state.cgb_game;
        candidate.cgb_revision_d = state.cgb_revision_d;
        candidate.stat_signal = state.stat_signal;
        candidate.stat_forced = state.stat_forced;
        candidate.vblank_if_countdown = state.vblank_if_countdown;
        candidate.lcd_on_delay = state.lcd_on_delay;
        candidate.lcd_on_hblank_extra = state.lcd_on_hblank_extra;
        candidate.ly_for_comparison = state.ly_for_comparison;
        candidate.lcd_on_short_line = state.lcd_on_short_line;
        candidate.mode3_scx_penalty = state.mode3_scx_penalty;
        candidate.mode3_sprite_penalty = state.mode3_sprite_penalty;
        candidate.wx_written_during_oam = state.wx_written_during_oam;
        candidate.accessed_oam_row = state.accessed_oam_row;
        *self = candidate;
        Ok(())
    }

    pub fn step(&mut self, cycles: u32) -> PpuStepResult {
        self.frame_complete = false;

        if self.lcdc & 0x80 == 0 {
            return self.step_lcd_off();
        }

        let mut lcd_stat = false;
        let mut vblank = false;

        for _ in 0..cycles {
            if self.lcd_on_delay > 0 {
                self.step_lcd_powering_on(&mut lcd_stat);
                continue;
            }
            self.step_dot(&mut lcd_stat, &mut vblank);
        }
        PpuStepResult {
            frame_done: self.frame_complete,
            lcd_stat,
            vblank,
        }
    }

    fn step_lcd_off(&mut self) -> PpuStepResult {
        self.ly = 0;
        self.mode_clock = 0;
        self.mode3_pipeline = None;
        // LCD off: STAT mode bits read as 00 (HBlank); the LYC
        // coincidence bit (bit2) is latched and retained.
        self.stat &= !0x03;
        PpuStepResult {
            frame_done: false,
            lcd_stat: false,
            vblank: false,
        }
    }

    fn step_lcd_powering_on(&mut self, lcd_stat: &mut bool) {
        // PPU is powering on: STAT mode bits stay at 00, but the
        // LYC comparison clock starts immediately (LY=0 was latched
        // by the LCDC write) so a coincident LY==LYC can fire the
        // STAT interrupt right away.
        self.lcd_on_delay -= 1;
        self.stat &= !0x03;
        self.refresh_lyc_flag();
        // The LCD-on forced pulse (stat_forced) only applies once
        // the PPU actually starts the first line.
        let signal = self.stat_signal_level();
        if signal && !self.stat_signal {
            *lcd_stat = true;
        }
        self.stat_signal = signal;
        if self.lcd_on_delay == 0 {
            // The first line starts at mode 2 (OAM search), so the
            // mode-2 STAT condition is active right away. Raise the
            // mode bits and evaluate once more at the handoff.
            self.stat = (self.stat & 0xFC) | PpuMode::OamSearch as u8;
            self.refresh_lyc_flag();
            let signal = self.stat_signal_level();
            if signal && !self.stat_signal {
                *lcd_stat = true;
            }
            self.stat_signal = signal;
        }
    }

    /// Evaluate the combined STAT interrupt signal and request the LCD STAT
    /// interrupt on a rising edge.
    fn eval_stat_signal(&mut self, lcd_stat: &mut bool) {
        let signal = self.stat_forced || self.stat_signal_level();
        if signal && !self.stat_signal {
            *lcd_stat = true;
        }
        self.stat_signal = signal;
    }

    /// The combined STAT interrupt signal level (TCAGBD 8.7):
    ///   (LY=LYC && bit 6) || (mode 0 && bit 3) || (mode 2 && bit 5)
    ///   || (mode 1 && (bit 4 || bit 5))
    /// The mode used here is the internal "mode for interrupt", which differs
    /// slightly from the mode visible in STAT reads: HBlank extends a few
    /// T-cycles into the following line (so the signal does not drop between
    /// HBlank and OAM search), VBlank starts 4 T-cycles into line 144, and
    /// the mode-2 condition also pulses during VBlank lines (low for the
    /// first 4 T-cycles of each VBlank line, then high — except on CGB
    /// hardware, where it is high from clock 0 of line 144 so the mode-2
    /// STAT fires one M-cycle before the VBL interrupt).
    fn stat_signal_level(&self) -> bool {
        if self.lcdc & 0x80 == 0 {
            return false;
        }
        if self.stat & 0x40 != 0 && self.lyc_coincide() {
            return true;
        }
        if self.lcd_on_delay > 0 {
            // Powering on: only the LYC path is active.
            return false;
        }
        match self.mode_for_interrupt() {
            PpuMode::HBlank => self.stat & 0x08 != 0,
            PpuMode::VBlank => self.stat_signal_vblank(),
            PpuMode::OamSearch => self.stat & 0x20 != 0,
            PpuMode::PixelTransfer => false,
        }
    }

    fn stat_signal_vblank(&self) -> bool {
        // The mode-1 condition is level-active for all of VBlank
        // (starting 4 T-cycles into line 144).
        if self.stat & 0x10 != 0 {
            return true;
        }
        // The mode-2 (OAM) condition pulses during VBlank: low for
        // the first few T-cycles of each line, then high.
        if self.stat & 0x20 != 0 {
            let rise = if self.ly == VBLANK_START {
                if self.cgb_mode { 0 } else { 4 }
            } else {
                Self::vblank_oam_pulse_rise()
            };
            return self.mode_clock >= rise;
        }
        false
    }

    /// The internal mode driving the STAT interrupt signal. HBlank extends
    /// into the start of the next line (until the mode-2 condition rises);
    /// VBlank starts 4 T-cycles into line 144 (on CGB hardware the mode-2
    /// condition fires at the very start of line 144 instead).
    fn mode_for_interrupt(&self) -> PpuMode {
        if self.ly >= VBLANK_START {
            if !self.cgb_mode && self.ly == VBLANK_START && self.mode_clock < 4 {
                PpuMode::HBlank
            } else {
                PpuMode::VBlank
            }
        } else if self.mode_clock < self.oam_irq_rise() {
            PpuMode::HBlank
        } else if self.mode_clock <= self.oam_search_cycles() {
            PpuMode::OamSearch
        } else if self.mode_clock < self.mode3_end_clock()
            || self
                .mode3_pipeline
                .as_ref()
                .is_some_and(Mode3Pipeline::unstarted_visible_sprite_pending)
        {
            PpuMode::PixelTransfer
        } else {
            PpuMode::HBlank
        }
    }

    /// T-cycle within the line at which the mode-2 STAT condition rises.
    fn oam_irq_rise(&self) -> u32 {
        if self.ly == 0 { 5 } else { 1 }
    }

    /// T-cycle at which the mode-2 condition rises on VBlank lines 145-153.
    /// Hardware measurement (mooneye intr_1_2_timing-GS) places the pulse
    /// 12 T-cycles into the line.
    fn vblank_oam_pulse_rise() -> u32 {
        12
    }

    /// T-cycles between LY becoming 144 and the VBL IF being set.
    fn vblank_if_delay() -> u8 {
        4
    }

    /// DMG LCD power-on delay (T-cycles).
    fn lcd_on_delay_dmg() -> u32 {
        77
    }

    /// T-cycles subtracted from the first line after the LCD is turned on
    /// (DMG): the first line is 81 T-cycles shorter than a normal scanline.
    fn lcd_on_short_dmg() -> u32 {
        81
    }

    /// T-cycle at which mode 0 (HBlank) starts: end of mode 3, which is
    /// extended by the fine-scroll penalty and sprite-fetch stalls.
    fn mode3_end_clock(&self) -> u32 {
        self.oam_search_cycles()
            + T_CYCLES_PIXEL_TRANSFER
            + self.mode3_scx_penalty
            + self
                .mode3_pipeline
                .as_ref()
                .map_or(self.mode3_sprite_penalty, |p| {
                    u32::from(p.sprite_extra_dots()).max(self.mode3_sprite_penalty)
                })
    }

    /// The LY=LYC coincidence signal feeding the STAT interrupt.
    fn lyc_coincide(&self) -> bool {
        if !self.cgb_mode && self.ly_for_comparison < 0 {
            false
        } else if self.cgb_mode {
            self.ly == self.lyc
        } else {
            self.ly_for_comparison as u16 == u16::from(self.lyc)
        }
    }

    /// Refresh the visible LY=LYC coincidence flag (STAT bit 2).
    fn refresh_lyc_flag(&mut self) {
        let coincide = self.lyc_coincide();
        let bit2 = if coincide { 0x04 } else { 0x00 };
        self.stat = (self.stat & !0x04) | bit2;
        self.prev_lyc_coincide = coincide;
    }

    fn step_dot(&mut self, lcd_stat: &mut bool, vblank: &mut bool) {
        if self.vblank_if_countdown != 0 {
            self.vblank_if_countdown -= 1;
            if self.vblank_if_countdown == 0 {
                *vblank = true;
            }
        }
        self.mode_clock += 1;
        if !self.cgb_mode {
            if self.lcd_on_hblank_extra > 0 {
                self.lcd_on_hblank_extra -= 1;
            }
            if self.ly_for_comparison < 0 && self.mode_clock >= 1 {
                self.ly_for_comparison = i16::from(self.ly);
            }
        }

        self.start_pipeline_if_needed();
        self.step_pipeline();
        self.advance_scanline();
        // Update the visible STAT mode bits; mode 3 clears the LCD-on
        // forced STAT pulse (after this dot's signal evaluation).
        let current_mode = self.current_mode();
        self.stat = (self.stat & 0xFC) | current_mode as u8;
        self.refresh_lyc_flag();
        self.eval_stat_signal(lcd_stat);
        if current_mode == PpuMode::PixelTransfer {
            self.stat_forced = false;
        }
        self.track_oam_row();
    }

    fn track_oam_row(&mut self) {
        if self.cgb_mode {
            self.accessed_oam_row = 0xFF;
        } else if self.lcdc & 0x80 != 0
            && self.ly < VBLANK_START
            && self.mode_clock < self.oam_search_cycles()
        {
            let row = ((self.mode_clock / 2) & !1) as u8 * 4 + 8;
            self.accessed_oam_row = if row + 8 <= 160 { row } else { 0xFF };
        } else {
            self.accessed_oam_row = 0xFF;
        }
    }

    fn start_pipeline_if_needed(&mut self) {
        if self.ly < VBLANK_START && self.mode_clock == self.oam_search_cycles() + 1 {
            let sprites = self.scanline_sprites();
            // The fetcher pays the raw SCX fine-scroll delay, but the CPU
            // observes the mode-3→0 boundary on 4-T-cycle bus phases. Round
            // the STAT/access boundary up to 0/4/8 T-cycles (mooneye
            // hblank_ly_scx_timing-GS).
            self.mode3_scx_penalty = u32::from((self.scx & 7).div_ceil(4) * 4);
            self.mode3_sprite_penalty = 0;
            let mut pipeline = Mode3Pipeline::new(
                Registers {
                    lcdc: self.lcdc,
                    scy: self.scy,
                    scx: self.scx,
                    wy: self.wy,
                    wx: self.wx,
                    bgp: self.bgp,
                    obp0: self.obp0,
                    obp1: self.obp1,
                },
                self.ly,
                self.window_line,
                self.window_eligible,
                sprites,
                self.cgb_mode,
                self.cgb_game,
                self.cgb_revision_d,
                self.opri,
            );
            pipeline.set_wx_written_during_oam(self.wx_written_during_oam);
            self.wx_written_during_oam = false;
            self.mode3_pipeline = Some(pipeline);
        }
    }

    fn step_pipeline(&mut self) {
        if let Some(pipeline) = self.mode3_pipeline.as_mut()
            && !pipeline.complete()
        {
            if let Some(output) = pipeline.step(&self.vram, &self.bg_palette, &self.obj_palette) {
                self.frame_buffer[usize::from(self.ly) * 160 + usize::from(output.x)] =
                    output.color;
            }
            self.mode3_sprite_penalty = u32::from(pipeline.sprite_extra_dots());
            if pipeline.complete() {
                self.window_line = pipeline.final_window_line();
                self.mode3_pipeline = None;
            }
        }
    }

    fn advance_scanline(&mut self) {
        let line_length = self.line_length();
        if self.mode_clock >= line_length {
            self.mode_clock -= line_length;
            self.mode3_pipeline = None;
            self.wx_written_during_oam = false;
            self.lcd_on_short_line = false;
            self.ly = self.ly.wrapping_add(1);
            if !self.cgb_mode {
                self.ly_for_comparison = -1;
                self.lcd_on_hblank_extra = 4;
            }
            if self.ly == VBLANK_START {
                // The VBL IF is set 4 T-cycles after LY becomes 144.
                self.vblank_if_countdown = Self::vblank_if_delay();
            }
            if self.ly >= SCANLINES_PER_FRAME {
                self.ly = 0;
                self.frame_complete = true;
                self.window_line = 0;
            }
            self.window_eligible = self.lcdc & 0x20 != 0 && self.ly >= self.wy;
        }
    }

    fn line_length(&self) -> u32 {
        if self.lcd_on_short_line {
            if self.cgb_mode {
                432
            } else {
                T_CYCLES_PER_SCANLINE - Self::lcd_on_short_dmg()
            }
        } else {
            T_CYCLES_PER_SCANLINE
        }
    }

    /// On DMG, the CPU-visible LY latch changes one T-cycle before the PPU's
    /// internal line state advances. Keep rendering/LYC on the internal line,
    /// but expose the next line during that final bus-access window.
    fn visible_ly(&self) -> u8 {
        if !self.cgb_mode
            && self.lcdc & 0x80 != 0
            && self.lcd_on_delay == 0
            && self.mode_clock + 1 >= self.line_length()
        {
            let next = self.ly.wrapping_add(1);
            if next >= SCANLINES_PER_FRAME { 0 } else { next }
        } else {
            self.ly
        }
    }

    fn current_mode(&self) -> PpuMode {
        if self.ly >= VBLANK_START {
            PpuMode::VBlank
        } else if self.lcd_on_delay > 0 {
            PpuMode::HBlank
        } else if self.mode_clock <= self.oam_search_cycles() {
            PpuMode::OamSearch
        } else if self.mode_clock <= self.mode3_end_clock()
            || self
                .mode3_pipeline
                .as_ref()
                .is_some_and(Mode3Pipeline::unstarted_visible_sprite_pending)
        {
            PpuMode::PixelTransfer
        } else {
            PpuMode::HBlank
        }
    }

    /// Mode as shown on STAT reads. During the brief HBlank continuation at
    /// the start of a scanline (a DMG LCD quirk), the mode bits read as 0 even
    /// though the real mode has already advanced to OAM search.
    fn stat_mode(&self) -> u8 {
        let mut stat = (self.stat & 0x78) | 0x80 | (self.stat & 0x07);
        if self.ly <= VBLANK_START && self.lcd_on_hblank_extra > 0 {
            stat &= 0xFC;
        }
        stat
    }

    /// Mode 2 is 76 T-cycles (not 80) on the first line after the LCD is
    /// turned on; on the DMG there is no OAM search on that line at all.
    fn oam_search_cycles(&self) -> u32 {
        if self.lcd_on_short_line {
            if self.cgb_mode { 76 } else { 0 }
        } else {
            T_CYCLES_OAM_SEARCH
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

    fn scanline_sprites(&self) -> Vec<Sprite> {
        let sprite_height = if self.lcdc & 0x04 != 0 { 16 } else { 8 };
        let mut sprites = Vec::with_capacity(10);
        for index in 0..40 {
            let top = i16::from(self.oam[index * 4]) - 16;
            if i16::from(self.ly) >= top && i16::from(self.ly) < top + sprite_height {
                sprites.push(Sprite {
                    x: i16::from(self.oam[index * 4 + 1]) - 8,
                    tile: self.oam[index * 4 + 2],
                    y: top,
                    flags: self.oam[index * 4 + 3],
                    oam_index: index as u8,
                });
                if sprites.len() == 10 {
                    break;
                }
            }
        }
        sprites
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

    pub(crate) fn set_dmg_compatibility_palettes(
        &mut self,
        palettes: crate::compatibility_palette::CompatibilityPalettes,
    ) {
        self.bg_palette[..4].copy_from_slice(&palettes.bg);
        self.obj_palette[..4].copy_from_slice(&palettes.obj0);
        self.obj_palette[4..8].copy_from_slice(&palettes.obj1);
    }

    /// Seed the PPU's frame phase (LY / T-cycle offset into the line) at
    /// power-on. The boot ROM runs with the LCD enabled, so by the time the
    /// game code starts the PPU is mid-frame; its exact position depends on
    /// the boot duration of the specific hardware model. `phase` is the
    /// number of T-cycles into the frame (mod 70224).
    pub fn set_frame_phase(&mut self, phase: u32) {
        let phase = phase % 70224;
        self.ly = (phase / T_CYCLES_PER_SCANLINE) as u8;
        self.mode_clock = phase % T_CYCLES_PER_SCANLINE;
        self.stat_signal = self.stat_signal_level();
        self.prev_lyc_coincide = self.ly == self.lyc;
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

    pub fn read_vram(&self, addr: u16) -> u8 {
        if !self.cgb_mode
            && self.lcdc & 0x80 != 0
            && self.lcd_on_delay == 0
            && self.mode_clock >= self.oam_search_cycles()
            && self.mode_clock <= self.mode3_end_clock()
        {
            return 0xFF;
        }
        let idx = if self.vbk == 0 {
            addr & 0x1FFF
        } else {
            0x2000 + (addr & 0x1FFF)
        };
        self.vram[idx as usize]
    }

    pub fn write_vram(&mut self, addr: u16, value: u8) {
        if !self.cgb_mode
            && self.lcdc & 0x80 != 0
            && self.lcd_on_delay == 0
            && self.mode_clock > self.oam_search_cycles()
            && self.mode_clock <= self.mode3_end_clock()
        {
            return;
        }
        let idx = if self.vbk == 0 {
            addr & 0x1FFF
        } else {
            0x2000 + (addr & 0x1FFF)
        };
        self.vram[idx as usize] = value;
    }

    pub fn read_oam(&self, addr: u8) -> u8 {
        // OAM is blocked (reads return 0xFF) during mode 2 and 3; it is only
        // readable during HBlank and VBlank. On the CGB the mode decides;
        // on the DMG the scanline position is used directly so that the
        // OAM-search window (which starts right after the line's brief
        // HBlank continuation) blocks reads.
        if self.lcdc & 0x80 != 0 && self.lcd_on_delay == 0 {
            let blocked = if self.cgb_mode {
                matches!(
                    self.current_mode(),
                    PpuMode::OamSearch | PpuMode::PixelTransfer
                )
            } else {
                self.mode_clock <= self.mode3_end_clock()
            };
            if blocked {
                return 0xFF;
            }
        }
        self.oam[addr as usize]
    }

    pub fn write_oam(&mut self, addr: u8, value: u8) {
        let blocked = !self.cgb_mode
            && self.lcdc & 0x80 != 0
            && self.lcd_on_delay == 0
            && self.mode_clock > 0
            && self.mode_clock != self.oam_search_cycles()
            && self.mode_clock <= self.mode3_end_clock();
        if blocked {
            return;
        }
        self.oam[addr as usize] = value;
    }

    /// OAM DMA writes are never blocked (the DMA is a PPU-internal access).
    pub fn dma_write_oam(&mut self, addr: u8, value: u8) {
        self.oam[addr as usize] = value;
    }

    /// DMG OAM bug. `kind` selects the pandocs-documented corruption pattern:
    /// a 16-bit register operation placing a value in $FE00-$FEFF on the
    /// address bus during OAM search glitches the 8-byte OAM row currently
    /// being scanned. The accessed address (beyond selecting $FExx) has no
    /// effect. `cycles_before_end` is the number of M-cycles between this
    /// access and the end of the instruction, shifting the scanned row back.
    /// Returns true if corruption happened.
    pub fn trigger_oam_bug(
        &mut self,
        address: u16,
        kind: OamBugKind,
        cycles_before_end: i16,
    ) -> bool {
        if self.cgb_mode || self.lcdc & 0x80 == 0 {
            return false;
        }
        if self.current_mode() != PpuMode::OamSearch {
            return false;
        }
        if address & 0xFF00 != 0xFE00 {
            return false;
        }
        // The row currently being scanned, as an 8-byte OAM index. The PPU
        // scans rows 0..19 during the 20 M-cycles of OAM search; the first
        // row is not corruptible (access lands one row ahead of the scan
        // pointer on real hardware).
        let row = (usize::from(self.accessed_oam_row) / 8) as i16 - cycles_before_end;
        self.oam_bug_corrupt(kind, row)
    }

    fn oam_bug_corrupt(&mut self, kind: OamBugKind, row: i16) -> bool {
        if !(1..=19).contains(&row) {
            return false;
        }
        let row = row as usize;
        let word_at = |oam: &[u8], off: usize| u16::from_le_bytes([oam[off], oam[off + 1]]);
        let mut kind = kind;
        if kind == OamBugKind::ReadInc {
            // Read combined with an increment/decrement: the first word of
            // the preceding row is glitched, then the preceding row is
            // copied both to the current row and two rows back.
            if (4..=18).contains(&row) {
                let a = word_at(&self.oam, (row - 2) * 8);
                let b = word_at(&self.oam, (row - 1) * 8);
                let c = word_at(&self.oam, row * 8);
                let d = word_at(&self.oam, (row - 1) * 8 + 4);
                let glitch = (b & (a | c | d)) | (a & c & d);
                self.oam[(row - 1) * 8] = (glitch & 0xFF) as u8;
                self.oam[(row - 1) * 8 + 1] = (glitch >> 8) as u8;
                let prev_row = self.oam[(row - 1) * 8..(row - 1) * 8 + 8].to_vec();
                self.oam[(row - 2) * 8..(row - 2) * 8 + 8].copy_from_slice(&prev_row);
                self.oam[row * 8..row * 8 + 8].copy_from_slice(&prev_row);
            }
            kind = OamBugKind::Read;
        }
        // a = first word of the current row, b = first word of the preceding
        // row, c = third word of the preceding row. The current row's first
        // word is glitched and its last three words are copied from the
        // preceding row.
        let a = word_at(&self.oam, row * 8);
        let b = word_at(&self.oam, (row - 1) * 8);
        let c = word_at(&self.oam, (row - 1) * 8 + 4);
        let corrupted = match kind {
            OamBugKind::Write => ((a ^ c) & (b ^ c)) ^ c,
            _ => b | (a & c),
        };
        self.oam[row * 8] = (corrupted & 0xFF) as u8;
        self.oam[row * 8 + 1] = (corrupted >> 8) as u8;
        let prev_tail = self.oam[(row - 1) * 8 + 2..(row - 1) * 8 + 8].to_vec();
        self.oam[row * 8 + 2..row * 8 + 8].copy_from_slice(&prev_tail);
        true
    }

    pub fn read_register(&self, addr: u16) -> u8 {
        match addr {
            0xFF40 => self.lcdc,
            0xFF41 => self.stat_mode(),
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.visible_ly(),
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,
            0xFF4F => 0xFE | self.vbk,
            0xFF68 => self.bgpi | 0x40,
            0xFF69 => {
                if self.current_mode() == PpuMode::PixelTransfer {
                    0xFF
                } else {
                    let idx = (self.bgpi & 0x3F) as usize;
                    let pal = self.bg_palette[idx >> 1];
                    if idx & 1 == 0 {
                        pal as u8
                    } else {
                        (pal >> 8) as u8
                    }
                }
            }
            0xFF6A => self.obpi | 0x40,
            0xFF6B => {
                if self.current_mode() == PpuMode::PixelTransfer {
                    0xFF
                } else {
                    let idx = (self.obpi & 0x3F) as usize;
                    let pal = self.obj_palette[idx >> 1];
                    if idx & 1 == 0 {
                        pal as u8
                    } else {
                        (pal >> 8) as u8
                    }
                }
            }
            0xFF6C => 0xFF,
            _ => 0xFF,
        }
    }

    pub fn write_register(&mut self, addr: u16, value: u8) {
        self.dispatch_pipeline_write(addr, value);
        match addr {
            0xFF40 => self.write_lcdc(value),
            0xFF41 => {
                self.stat = (self.stat & 0x07) | (value & 0x78);
            }
            0xFF42 => self.scy = value,
            0xFF43 => self.scx = value,
            0xFF45 => {
                self.lyc = value;
                // Writing LYC updates the LY=LYC comparison immediately
                // while the LCD is on. When off, the comparison clock is
                // stopped and the coincidence bit is latched.
                if self.lcdc & 0x80 != 0 {
                    self.refresh_lyc_flag();
                }
            }
            0xFF47 => self.bgp = value,
            0xFF48 => self.obp0 = value,
            0xFF49 => self.obp1 = value,
            0xFF4A => self.wy = value,
            0xFF4B => {
                if self.current_mode() == PpuMode::OamSearch {
                    self.wx_written_during_oam = true;
                }
                self.wx = value;
            }
            0xFF4F => self.vbk = value & 0x01,
            0xFF68 => self.write_bgpi(value),
            0xFF69 => self.write_bg_palette_data(value),
            0xFF6A => self.write_obpi(value),
            0xFF6B => self.write_obj_palette_data(value),
            0xFF6C => self.set_key0(value),
            _ => {}
        }
    }

    fn dispatch_pipeline_write(&mut self, addr: u16, value: u8) {
        if matches!(
            addr,
            0xFF40 | 0xFF42 | 0xFF43 | 0xFF47 | 0xFF48 | 0xFF49 | 0xFF4A | 0xFF4B
        ) && let Some(pipeline) = self.mode3_pipeline.as_mut()
        {
            pipeline.write_register(addr, value);
            if let Some(output) = pipeline.take_corrected_output() {
                self.frame_buffer[usize::from(self.ly) * 160 + usize::from(output.x)] =
                    output.color;
            }
        }
    }

    fn write_lcdc(&mut self, value: u8) {
        let lcd_was_enabled = self.lcdc & 0x80 != 0;
        self.lcdc = value;
        if !lcd_was_enabled && value & 0x80 != 0 {
            // The PPU starts at the beginning of the next scanline. During
            // the power-on delay (20 T-cycles) STAT mode bits read as 00 and
            // the LYC comparison clock is not running.
            self.lcd_on_delay = if self.cgb_mode { 20 } else { 77 };
            self.lcd_on_short_line = true;
            self.ly_for_comparison = 0;
            self.window_eligible = value & 0x20 != 0 && self.ly >= self.wy;
            // LY=0 at power-on. The comparison against LYC is evaluated on
            // the first step() during the power-on delay. Keep the previous
            // coincidence state so only a false->true transition (e.g. after
            // the comparison was false while off) raises a new interrupt.
            self.ly = 0;
            self.stat &= !0x03;
            // A mode-2 STAT interrupt pulse right after the LCD turns on
            // when the OAM interrupt is enabled.
            if self.stat & 0x20 != 0 {
                self.stat_forced = true;
            }
        }
    }

    fn write_bgpi(&mut self, value: u8) {
        // BCPS index/auto-increment always updates, even in DMG emulation
        // mode (only the palette data writes below are gated by KEY0 bit 2).
        self.bgpi = value & 0x3F;
        if value & 0x80 != 0 {
            self.bgpi |= 0x80;
        }
    }

    fn write_obpi(&mut self, value: u8) {
        self.obpi = value & 0x3F;
        if value & 0x80 != 0 {
            self.obpi |= 0x80;
        }
    }

    fn write_bg_palette_data(&mut self, value: u8) {
        if self.key0 & 0x04 != 0 {
            return;
        }
        let idx = (self.bgpi & 0x3F) as usize;
        let auto_inc = self.bgpi & 0x80 != 0;
        if idx & 1 == 0 {
            self.bg_palette[idx >> 1] = (self.bg_palette[idx >> 1] & 0xFF00) | value as u16;
        } else {
            self.bg_palette[idx >> 1] = (self.bg_palette[idx >> 1] & 0x00FF) | (value as u16) << 8;
        }
        if auto_inc {
            self.bgpi = (self.bgpi & 0x80) | ((self.bgpi + 1) & 0x3F);
        }
    }

    fn write_obj_palette_data(&mut self, value: u8) {
        if self.key0 & 0x04 != 0 {
            return;
        }
        let idx = (self.obpi & 0x3F) as usize;
        let auto_inc = self.obpi & 0x80 != 0;
        if idx & 1 == 0 {
            self.obj_palette[idx >> 1] = (self.obj_palette[idx >> 1] & 0xFF00) | value as u16;
        } else {
            self.obj_palette[idx >> 1] =
                (self.obj_palette[idx >> 1] & 0x00FF) | (value as u16) << 8;
        }
        if auto_inc {
            self.obpi = (self.obpi & 0x80) | ((self.obpi + 1) & 0x3F);
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
        // The VBL interrupt fires 4 T-cycles after LY becomes 144.
        assert!(!r.vblank);
        assert!(p.step(4).vblank);
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
        p.step(1);
        assert!(p.read_register(0xFF41) & 0x04 != 0);
    }

    #[test]
    fn lyc_coincidence_clears_when_ly_passes() {
        let mut p = ppu();
        p.write_register(0xFF45, 3);
        step_ly(&mut p, 3);
        p.step(1);
        assert!(p.read_register(0xFF41) & 0x04 != 0);
        step_ly(&mut p, 4);
        p.step(1);
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
        // LY becomes 144 at the end of this line; the VBL IF and the mode-1
        // STAT interrupt fire 4 T-cycles later.
        let r = p.step(T_CYCLES_PER_SCANLINE + 4);
        assert!(r.vblank);
        assert!(r.lcd_stat);
    }

    #[test]
    fn lyc_interrupt_fires_on_coincidence() {
        let mut p = ppu();
        p.write_register(0xFF41, 0x40);
        p.write_register(0xFF45, 5);
        step_ly(&mut p, 5);
        let r = p.step(1);
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

        // 4 T-cycles into line 144.
        assert!(p.step(T_CYCLES_PER_SCANLINE + 4).vblank);
        assert!(!p.step(T_CYCLES_PER_SCANLINE).vblank);
    }

    #[test]
    fn lcd_enable_fires_mode_2_stat_interrupt_on_line_zero() {
        let mut p = ppu();
        p.cgb_mode = true;
        p.write_register(0xFF41, 0x20);
        p.write_register(0xFF40, p.read_register(0xFF40) & !0x80);
        p.step(1);
        p.write_register(0xFF40, p.read_register(0xFF40) | 0x80);

        // LCD on: mode bits read as 00 during the power-on delay, then mode 2
        // (OAM search) starts on the first scanline and raises STAT.
        let result = p.step(1);
        assert!(!result.lcd_stat);
        assert_eq!(p.read_register(0xFF44), 0);

        let result = p.step(20);
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
    fn stat_mode_keeps_final_pixel_transfer_t_cycle_visible() {
        let mut p = ppu();
        p.mode_clock = p.mode3_end_clock() - 1;
        p.step(1);
        assert_eq!(p.read_register(0xFF41) & 0x03, 3);
        p.step(1);
        assert_eq!(p.read_register(0xFF41) & 0x03, 0);
    }

    #[test]
    fn dmg_scx_quantizes_cpu_visible_hblank_boundary() {
        let expected = [0, 4, 4, 4, 4, 8, 8, 8];
        for (scx, expected_penalty) in expected.into_iter().enumerate() {
            let mut p = ppu();
            p.scx = scx as u8;
            p.mode_clock = T_CYCLES_OAM_SEARCH + 1;
            p.start_pipeline_if_needed();
            assert_eq!(p.mode3_scx_penalty, expected_penalty, "SCX={scx}");
        }
    }

    #[test]
    fn dmg_ly_latch_exposes_next_line_on_final_t_cycle() {
        let mut p = ppu();
        p.ly = 65;
        p.mode_clock = T_CYCLES_PER_SCANLINE - 2;
        assert_eq!(p.read_register(0xFF44), 65);
        p.mode_clock += 1;
        assert_eq!(p.read_register(0xFF44), 66);
        assert_eq!(p.ly, 65, "internal line advances on the following T-cycle");
    }

    #[test]
    fn stat_irq_line_blocks_repeated_mode_interrupts() {
        let mut p = ppu();
        // Enable all mode interrupts, then advance one line so the mode-2
        // interrupt from the STAT write is consumed before reaching VBlank.
        p.write_register(0xFF41, 0x78);
        p.step(1);
        // Advance to LY=144 (VBlank, mode 1). The mode-1 transition fires a
        // STAT interrupt during the last scanline step and asserts the line.
        step_ly(&mut p, VBLANK_START);
        // While the line stays asserted (mode 1, no mode 3 yet), a mode
        // transition to HBlank on the next scanline must not re-fire.
        assert!(!p.step(T_CYCLES_PER_SCANLINE).lcd_stat);
    }

    #[test]
    fn stat_write_enabling_current_mode_fires_immediately() {
        let mut p = ppu();
        // Advance into VBlank (mode 1), then enable the mode 1 interrupt.
        for _ in 0..144 {
            p.step(T_CYCLES_PER_SCANLINE);
        }
        // Line 144's first 4 T-cycles still belong to the previous HBlank,
        // so the mode-1 condition is only met from T-cycle 4 on.
        p.step(1);
        p.write_register(0xFF41, 0x10);
        assert!(p.step(4).lcd_stat);
    }

    #[test]
    fn stat_mode_is_1_during_vblank() {
        let mut p = ppu();
        for _ in 0..144 {
            p.step(T_CYCLES_PER_SCANLINE);
        }
        // The mode bits read as 0 (HBlank) for the first 4 T-cycles of
        // line 144, then as 1 (VBlank).
        p.step(1);
        assert_eq!(p.read_register(0xFF41) & 0x03, 0);
        p.step(3);
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
