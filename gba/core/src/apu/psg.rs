/// GBA PSG channels; clocks are CPU T-cycles (16.78MHz square/wave/noise
/// periods with a 512Hz frame sequencer, per GBATEK).
const DUTY: [[i8; 8]; 4] = [
    [1, 0, 0, 0, 0, 0, 0, 1],
    [1, 1, 0, 0, 0, 0, 0, 1],
    [1, 1, 1, 1, 0, 0, 0, 0],
    [0, 0, 1, 1, 1, 1, 1, 1],
];

/// Shared length/envelope core for square/noise channels.
#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct LengthEnvelope {
    pub active: bool,
    pub length: u8,
    pub volume: u8,
    env_timer: u8,
}

impl LengthEnvelope {
    pub fn trigger(&mut self, init_len: u8, init_vol: u8, env_reg: u16, seq_odd: bool) {
        if self.length == 0 {
            self.length = init_len;
        }
        // Retrigger quirk: a trigger landing just before a length step
        // consumes one extra tick immediately.
        if seq_odd {
            self.length = self.length.saturating_sub(1);
        }
        self.volume = init_vol;
        self.env_timer = (env_reg >> 8) as u8 & 7;
        self.active = self.length != 0;
    }

    pub fn tick_length(&mut self, enabled: bool) {
        if !enabled || self.length == 0 {
            return;
        }
        self.length -= 1;
        if self.length == 0 {
            self.active = false;
        }
    }

    pub fn tick_envelope(&mut self, env_reg: u16) {
        if !self.active {
            return;
        }
        // Saturated envelopes go dead: volume holds, ticking stops
        // (output matches either way: clamped at 15/0).
        let inc = env_reg & (1 << 11) != 0;
        if (inc && self.volume >= 15) || (!inc && self.volume == 0) {
            return;
        }
        let pace = (env_reg >> 8) as u8 & 7;
        if pace == 0 {
            return;
        }
        if self.env_timer == 0 {
            self.env_timer = pace;
        }
        self.env_timer -= 1;
        if self.env_timer == 0 {
            self.env_timer = pace;
            if env_reg & (1 << 11) != 0 {
                if self.volume < 15 {
                    self.volume += 1;
                }
            } else if self.volume > 0 {
                self.volume -= 1;
            }
        }
    }
}

/// Square channel (ch1 adds sweep).
#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Square {
    pub core: LengthEnvelope,
    pub freq_shadow: u16,
    timer: u32,
    phase: u8,
    sweep_shift: u8,
    sweep_pace: u8,
    sweep_timer: u8,
    sweep_dir_dec: bool,
    sweep_occurred: bool,
}

impl Square {
    pub fn trigger(&mut self, freq: u16, init_len: u8, init_vol: u8, env_reg: u16, seq_odd: bool) {
        self.freq_shadow = freq;
        self.core.trigger(init_len, init_vol, env_reg, seq_odd);
        self.timer = 0;
        self.phase = 0;
        self.sweep_timer = self.sweep_pace;
        self.sweep_occurred = false;
        // Immediate overflow check when sweep is armed with a shift.
        if self.sweep_active() {
            self.sweep_calc(true);
        }
    }

    fn sweep_active(&self) -> bool {
        self.sweep_pace != 8 || self.sweep_shift != 0
    }

    /// NR10 write (GBATEK sweep, incl. direction-flip zombie rule).
    pub fn write_sweep(&mut self, value: u8) {
        let shift = value & 7;
        let dec = value & (1 << 3) != 0;
        let pace = (value >> 4) & 7;
        if self.sweep_occurred && self.sweep_dir_dec && !dec {
            self.core.active = false;
        }
        self.sweep_shift = shift;
        self.sweep_dir_dec = dec;
        // Pace 0 decodes to internal 8 ("off" code).
        self.sweep_pace = if pace == 0 { 8 } else { pace };
    }

    /// `freq` is the live register value for ch2; ch1 (sweep) uses the
    /// shadow once the sweep unit has run.
    pub fn tick_timer(&mut self, freq: u16, use_shadow: bool) {
        if !self.core.active {
            return;
        }
        if self.timer == 0 {
            let base = if use_shadow {
                self.freq_shadow & 0x7FF
            } else {
                freq & 0x7FF
            };
            self.timer = 16 * u32::from(2048 - base.min(2047));
            self.phase = (self.phase + 1) & 7;
        }
        self.timer -= 1;
    }

    /// Frame-sequencer sweep steps (2, 6). Returns false when the sweep
    /// overflows and kills the channel.
    pub fn tick_sweep(&mut self) -> bool {
        if !self.core.active {
            return true;
        }
        if self.sweep_timer == 0 {
            self.sweep_timer = self.sweep_pace;
        }
        self.sweep_timer -= 1;
        if self.sweep_timer == 0 {
            self.sweep_timer = self.sweep_pace;
            if self.sweep_pace != 8 {
                return self.sweep_calc(false);
            }
        }
        true
    }

    fn sweep_calc(&mut self, initial: bool) -> bool {
        let shift = self.sweep_shift;
        if shift == 0 {
            return true;
        }
        let offset = self.freq_shadow & 0x7FF;
        let delta = offset >> shift;
        let next = if self.sweep_dir_dec {
            self.freq_shadow.wrapping_sub(delta) & 0x7FF
        } else {
            let next = offset + delta;
            if next >= 2048 {
                self.core.active = false;
                return false;
            }
            next
        };
        if !initial || !self.sweep_dir_dec {
            self.freq_shadow = (self.freq_shadow & !(0x7FF)) | next;
        }
        // Increment mode runs the overflow check a second time at once.
        if !self.sweep_dir_dec && !initial {
            let next2 = next + (next >> shift);
            if next2 >= 2048 {
                self.core.active = false;
                return false;
            }
        }
        self.sweep_occurred = true;
        true
    }

    pub fn output(&self, duty: u8) -> i16 {
        if !self.core.active {
            return 0;
        }
        let high = DUTY[(duty & 3) as usize][self.phase as usize] != 0;
        let vol = i16::from(self.core.volume);
        if high { vol } else { -vol }
    }
}

/// Wave channel (ch3).
#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Wave {
    pub active: bool,
    pub length: u16,
    timer: u32,
    pub phase: u8,
    pub bank: usize,
    pub dimension_64: bool,
    pub hold: u8,
}

impl Wave {
    pub fn trigger(&mut self, init_len: u16, dimension_64: bool, seq_odd: bool) {
        if self.length == 0 {
            self.length = init_len;
        }
        if seq_odd {
            self.length = self.length.saturating_sub(1);
        }
        self.active = self.length != 0;
        self.timer = 0;
        self.phase = 0;
        if dimension_64 {
            self.bank = 0;
        }
        // Triggered retrigger latches the first nibble immediately.
    }

    pub fn tick_timer(&mut self, rate: u16) {
        if !self.active {
            return;
        }
        if self.timer == 0 {
            self.timer = 8 * u32::from(2048 - rate.min(2047));
            self.phase = (self.phase + 1) & 31;
            if self.phase == 0 && self.dimension_64 {
                // 64-digit mode alternates banks every 32 digits.
                self.bank ^= 1;
            }
        }
        self.timer -= 1;
    }

    pub fn tick_length(&mut self, enabled: bool) {
        if !enabled || self.length == 0 {
            return;
        }
        self.length -= 1;
        if self.length == 0 {
            self.active = false;
        }
    }

    /// Current digit nibble from the PLAYING bank; holds the last digit
    /// while disabled.
    pub fn nibble(&mut self, wave_ram: &[u8; 0x20]) -> u8 {
        if !self.active {
            return self.hold;
        }
        let byte = wave_ram[self.bank * 16 + (self.phase / 2) as usize];
        let nib = if self.phase & 1 == 0 {
            byte >> 4
        } else {
            byte & 0xF
        };
        self.hold = nib;
        nib
    }

    pub fn output(&mut self, wave_ram: &[u8; 0x20], vol_code: u8, force_75: bool) -> i16 {
        if !self.active {
            return 0;
        }
        let base = i16::from(self.nibble(wave_ram)) - 8;
        if force_75 {
            base * 3 / 4
        } else {
            match vol_code & 3 {
                0 => 0,
                1 => base,
                2 => base / 2,
                _ => base / 4,
            }
        }
    }
}

/// Noise channel (ch4).
#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Noise {
    pub core: LengthEnvelope,
    timer: u32,
    lfsr: u16,
    width_7: bool,
}

impl Noise {
    pub fn trigger(
        &mut self,
        init_len: u8,
        init_vol: u8,
        env_reg: u16,
        width_7: bool,
        seq_odd: bool,
    ) {
        self.core.trigger(init_len, init_vol, env_reg, seq_odd);
        self.width_7 = width_7;
        self.lfsr = if width_7 { 0x40 } else { 0x4000 };
        self.timer = 0;
    }

    pub fn tick_timer(&mut self, ratio: u8, shift: u8) {
        if !self.core.active {
            return;
        }
        if self.timer == 0 {
            // Interval form: (64 << shift), ratio 0 halves.
            let mut interval = 64u32 << shift.min(12);
            if ratio == 0 {
                interval /= 2;
            } else {
                interval *= u32::from(ratio);
            }
            self.timer = interval;
            let carry = self.lfsr & 1;
            self.lfsr >>= 1;
            if carry != 0 {
                self.lfsr ^= if self.width_7 { 0x60 } else { 0x6000 };
            }
        }
        self.timer -= 1;
    }

    pub fn output(&self) -> i16 {
        if !self.core.active {
            return 0;
        }
        // GBATEK: carry-out drives HIGH.
        let vol = i16::from(self.core.volume);
        if self.lfsr & 1 == 0 { vol } else { -vol }
    }
}

impl LengthEnvelope {
    /// Phase 10 import validation (bounds follow the trigger/write masks).
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.length > 64 {
            return Err(format!(
                "apu: envelope length out of range: {}",
                self.length
            ));
        }
        if self.volume > 15 {
            return Err(format!(
                "apu: envelope volume out of range: {}",
                self.volume
            ));
        }
        if self.env_timer > 7 {
            return Err(format!(
                "apu: envelope timer out of range: {}",
                self.env_timer
            ));
        }
        Ok(())
    }
}

impl Square {
    pub(super) fn validate(&self) -> Result<(), String> {
        self.core.validate()?;
        // `tick_timer`: 16 * (2048 - base), base 11-bit.
        if self.timer > 0x8000 {
            return Err(format!("apu: square timer out of range: {}", self.timer));
        }
        if self.phase > 7 {
            return Err(format!("apu: square phase out of range: {}", self.phase));
        }
        if self.sweep_shift > 7 {
            return Err(format!(
                "apu: sweep shift out of range: {}",
                self.sweep_shift
            ));
        }
        if self.sweep_pace > 8 {
            return Err(format!("apu: sweep pace out of range: {}", self.sweep_pace));
        }
        if self.sweep_timer > 8 {
            return Err(format!(
                "apu: sweep timer out of range: {}",
                self.sweep_timer
            ));
        }
        Ok(())
    }
}

impl Wave {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.length > 256 {
            return Err(format!("apu: wave length out of range: {}", self.length));
        }
        // `tick_timer`: 8 * (2048 - rate), rate 11-bit.
        if self.timer > 0x4000 {
            return Err(format!("apu: wave timer out of range: {}", self.timer));
        }
        if self.phase > 31 {
            return Err(format!("apu: wave phase out of range: {}", self.phase));
        }
        if self.bank > 1 {
            return Err(format!("apu: wave bank out of range: {}", self.bank));
        }
        if self.hold > 15 {
            return Err(format!("apu: wave hold out of range: {}", self.hold));
        }
        Ok(())
    }
}

impl Noise {
    pub(super) fn validate(&self) -> Result<(), String> {
        self.core.validate()?;
        // `tick_timer`: (64 << shift<=12) * ratio<=7.
        if self.timer > 0x200_000 {
            return Err(format!("apu: noise timer out of range: {}", self.timer));
        }
        if self.lfsr > 0x7FFF {
            return Err(format!("apu: noise lfsr out of range: {:#X}", self.lfsr));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_duty_output_tracks_phase_and_volume() {
        let mut sq = Square::default();
        sq.core.active = true;
        sq.core.volume = 12;
        sq.phase = 0;
        // Duty 2 (50%): phases 0-3 high.
        assert_eq!(sq.output(2), 12);
        sq.phase = 4;
        assert_eq!(sq.output(2), -12);
        sq.core.active = false;
        assert_eq!(sq.output(2), 0);
    }

    #[test]
    fn envelope_ticks_down_to_silence() {
        let mut le = LengthEnvelope::default();
        // env reg: pace 1, dec, init vol 2.
        le.trigger(64, 2, 0x0200 | 2 << 12, false);
        le.tick_envelope(0x0200 | 2 << 12);
        assert_eq!(le.volume, 2);
        le.tick_envelope(0x0200 | 2 << 12);
        assert_eq!(le.volume, 1);
        le.tick_envelope(0x0200 | 2 << 12);
        le.tick_envelope(0x0200 | 2 << 12);
        assert_eq!(le.volume, 0);
    }

    #[test]
    fn length_expiry_kills_channel() {
        let mut le = LengthEnvelope::default();
        le.trigger(2, 8, 0, false);
        assert!(le.active);
        le.tick_length(true);
        assert!(le.active);
        le.tick_length(true);
        assert!(!le.active);
    }

    #[test]
    fn sweep_increment_overflow_disables() {
        let mut sq = Square::default();
        sq.core.active = true;
        // freq near top, shift 1, inc, pace 1.
        sq.freq_shadow = 0x7F0;
        sq.sweep_shift = 1;
        sq.sweep_pace = 1;
        sq.sweep_timer = 1;
        sq.sweep_dir_dec = false;
        assert!(!sq.tick_sweep());
        assert!(!sq.core.active);
    }

    #[test]
    fn noise_lfsr_advances_and_resets() {
        let mut nz = Noise::default();
        nz.trigger(64, 10, 0, false, false);
        assert_eq!(nz.lfsr, 0x4000);
        nz.tick_timer(1, 3);
        // Timer expiry shifts immediately: bit0 was 0, so plain shift.
        assert_eq!(nz.lfsr, 0x2000);
    }

    #[test]
    fn wave_nibble_order_is_msb_first() {
        let mut w = Wave {
            active: true,
            ..Default::default()
        };
        w.phase = 0;
        let mut ram = [0u8; 0x20];
        ram[0] = 0xAB;
        assert_eq!(w.nibble(&ram), 0xA);
        w.phase = 1;
        assert_eq!(w.nibble(&ram), 0xB);
    }
}
