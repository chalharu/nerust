/// Square duty patterns (HW-pinned, mgba parity): 12.5% has a single
/// high step, 25% two, 50% four, 75% six. Phase 0 is the trigger start.
const DUTY: [[i8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 0],
];

/// PolyBLEP edge slots per square voice per grid period (12 output edges
/// need a sub-85-T-cycle phase step, i.e. fundamentals above ~24kHz;
/// extras saturate there, where the fundamental itself is inaudible).
const BLEP_MAX_EDGES: usize = 12;

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

    /// Obscure Behavior (mirrors GBC `reload_timer`): a trigger landing
    /// just before an envelope step reloads the timer with pace + 1.
    pub(crate) fn envelope_extra_tick(&mut self) {
        self.env_timer = self.env_timer.wrapping_add(1);
    }

    #[cfg(test)]
    pub(crate) fn env_timer_for_test(&self) -> u8 {
        self.env_timer
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
    /// PolyBLEP edge backlog for the grid period being accumulated:
    /// `(t, h)` = (fraction from period start, baked step height in
    /// naive output units). Transient (sub-sample timing): skipped by
    /// serde so save format and old saves are untouched; a load simply
    /// skips one retro tap (inaudible).
    #[serde(skip)]
    blep_edges: [(f32, f32); BLEP_MAX_EDGES],
    #[serde(skip)]
    blep_len: u8,
    /// False after a trigger/reconfig inside the current period: pending
    /// retro taps would mix pre/post-trigger state, so they are dropped
    /// once (after taps still apply).
    #[serde(skip)]
    blep_clean: bool,
}

impl Square {
    /// Batching support: first zero-hit is `timer + 1` ticks out (hit fires
    /// when the timer reads 0 at tick start), so `timer` upcoming ticks are
    /// hit-free. Interior advance only decrements; phase/bank stay put.
    pub(crate) fn timer_horizon(&self) -> Option<u32> {
        self.core.active.then_some(self.timer)
    }

    /// Decrement the phase timer. Valid only when no zero-hit occurs
    /// inside the span (verified by the horizon): plain `-=` matches the
    /// per-cycle behavior exactly, including dev-profile underflow panic.
    /// Inactive voices are untouched, mirroring the `tick_timer` early
    /// return.
    pub(crate) fn advance_timer(&mut self, n: u32) {
        if !self.core.active {
            return;
        }
        self.timer -= n;
    }

    pub fn trigger(&mut self, freq: u16, init_len: u8, init_vol: u8, env_reg: u16, seq_odd: bool) {
        self.freq_shadow = freq;
        self.core.trigger(init_len, init_vol, env_reg, seq_odd);
        // The duty step is kept across triggers (only its timer restarts,
        // Pan Docs + mGBA parity): the latched step plays a full period.
        self.timer = 16 * u32::from(2048 - freq.min(2047));
        self.sweep_timer = self.sweep_pace;
        self.sweep_occurred = false;
        self.clear_blep();
        // Immediate overflow check when sweep is armed with a shift.
        if self.sweep_active() {
            self.sweep_calc(true);
        }
    }

    fn sweep_active(&self) -> bool {
        self.sweep_pace != 8 || self.sweep_shift != 0
    }

    #[cfg(test)]
    pub(crate) fn sweep_pace_for_test(&self) -> u8 {
        self.sweep_pace
    }

    #[cfg(test)]
    pub(crate) fn phase_for_test(&self) -> u8 {
        self.phase
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
    /// shadow once the sweep unit has run. `duty` resolves output edges
    /// for polyBLEP recording; `mix_left` is the grid countdown remaining
    /// (1..=512), fixing each edge's fractional position in the period.
    pub fn tick_timer(&mut self, freq: u16, use_shadow: bool, duty: u8, mix_left: u32) {
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
            let from = self.phase;
            self.phase = (self.phase + 1) & 7;
            // Band-limited synthesis: a duty-boundary crossing is a step
            // discontinuity; naive per-sample synthesis folds its harmonics
            // back as inharmonic aliases (metallic harshness next to
            // blip_buf emulators). Record `(t, h)` for the Kleimola
            // 2-tap polyBLEP consumed at the next grid push.
            let pat = &DUTY[(duty & 3) as usize];
            let step = f32::from(pat[self.phase as usize] - pat[from as usize]);
            if step != 0.0 && (self.blep_len as usize) < BLEP_MAX_EDGES {
                let period = super::T_CYCLES_PER_MIX as f32;
                let t = (period - mix_left.min(super::T_CYCLES_PER_MIX as u32) as f32) / period;
                let h = step * 2.0 * f32::from(self.core.volume);
                self.blep_edges[self.blep_len as usize] = (t, h);
                self.blep_len += 1;
            }
        }
        self.timer -= 1;
    }

    /// Drain recorded polyBLEP edges for the grid sample being pushed.
    /// Returns `(after, retros, retro_len)`: `after` corrects the current
    /// sample (`-h/2 * t^2` per edge, naive output units); `retros` holds
    /// `(t, h)` pairs correcting the previously pushed sample with
    /// `+h/2 * (1-t)^2` (empty when a trigger/reconfig dirtied the
    /// period). Bookkeeping resets for the next period.
    pub(crate) fn take_blep(&mut self) -> (f32, [(f32, f32); BLEP_MAX_EDGES], u8) {
        let mut after = 0.0f32;
        let mut retros = [(0.0f32, 0.0f32); BLEP_MAX_EDGES];
        let mut retro_len = 0u8;
        for i in 0..self.blep_len as usize {
            let (t, h) = self.blep_edges[i];
            after += -h * 0.5 * t * t;
            if self.blep_clean {
                retros[retro_len as usize] = (t, h);
                retro_len += 1;
            }
        }
        self.blep_len = 0;
        self.blep_clean = true;
        (after, retros, retro_len)
    }

    /// Drop polyBLEP bookkeeping (trigger starts a fresh note; master-off
    /// parks the voices). Pending retro taps would mix states, so the
    /// next drain skips them once.
    pub(crate) fn clear_blep(&mut self) {
        self.blep_len = 0;
        self.blep_clean = false;
    }

    /// Frame-sequencer sweep steps (2, 6). Returns false when the sweep
    /// overflows and kills the channel. Pace 0 (NR10 never written) and    /// the pace-8 off-code both disable the unit: no timer movement, no
    /// calculation. (Without this, pace 0 would underflow `sweep_timer`
    /// on the reload-then-decrement below.)
    pub fn tick_sweep(&mut self) -> bool {
        if !self.core.active {
            return true;
        }
        if self.sweep_pace == 0 || self.sweep_pace == 8 {
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
    /// Batching support: see `Square::timer_horizon`.
    pub(crate) fn timer_horizon(&self) -> Option<u32> {
        self.active.then_some(self.timer)
    }

    /// Batching support: see `Square::advance_timer`.
    pub(crate) fn advance_timer(&mut self, n: u32) {
        if !self.active {
            return;
        }
        self.timer -= n;
    }

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
        // The start bank was latched from NR30 bit 6 by the caller:
        // GBATEK plays the selected bank first in 64-digit mode too.
        self.dimension_64 = dimension_64;
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
        // Unipolar 0..15 like hardware (and mGBA): DC rides along and the
        // output HPF strips it downstream. Full scale matches square
        // voices (nibble 15 ~= square vol 15).
        let base = i16::from(self.nibble(wave_ram));
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
    /// Batching support: see `Square::timer_horizon`. The LFSR only shifts
    /// on zero-hits, so capping at the first hit keeps it exact.
    pub(crate) fn timer_horizon(&self) -> Option<u32> {
        self.core.active.then_some(self.timer)
    }

    /// Batching support: see `Square::advance_timer`.
    pub(crate) fn advance_timer(&mut self, n: u32) {
        if !self.core.active {
            return;
        }
        self.timer -= n;
    }

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
            // Interval form: (64 << shift), ratio 0 halves. Shift is a
            // full 4 bits (0-15); all positions are defined dividers.
            let mut interval = 64u32 << shift.min(15);
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
        // 8 is producible: pace 7 reloaded with the pre-envelope-step
        // extra tick (not a corrupt import).
        if self.env_timer > 8 {
            return Err(format!(
                "apu: envelope timer out of range: {}",
                self.env_timer
            ));
        }
        Ok(())
    }
}

impl Square {
    /// Phase 10 import validation (bounds follow the trigger/write masks).
    /// `has_sweep` is ch1-only: ch2 shares this struct but has no sweep
    /// unit, so its sweep fields stay at the never-written zero and must
    /// not be policed (an active ch2 with pace 0 is everyday state).
    pub(super) fn validate(&self, has_sweep: bool) -> Result<(), String> {
        self.core.validate()?;
        // `tick_timer`: 16 * (2048 - base), base 11-bit.
        if self.timer > 0x8000 {
            return Err(format!("apu: square timer out of range: {}", self.timer));
        }
        if self.phase > 7 {
            return Err(format!("apu: square phase out of range: {}", self.phase));
        }
        if !has_sweep {
            return Ok(());
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
        // Pace 0 with no shift is the never-written shape (NR10 untouched:
        // sweep off); `tick_sweep` treats it as disabled, so a live voice
        // in that shape is legitimate. With a shift it is non-producible
        // and would arm the sweep path, so keep rejecting that.
        if self.core.active && self.sweep_pace == 0 && self.sweep_shift != 0 {
            return Err("apu: sounding channel with zero sweep pace".to_string());
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
        // `tick_timer`: (64 << shift<=15) * ratio<=7, max (64<<15)*7.
        if self.timer > 0xE00_000 {
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
        // Duty 0 (12.5%): only phase 7 high.
        sq.phase = 7;
        assert_eq!(sq.output(0), 12);
        sq.phase = 0;
        assert_eq!(sq.output(0), -12);
        // Duty 1 (25%): phases 0 and 7 high.
        sq.phase = 0;
        assert_eq!(sq.output(1), 12);
        sq.phase = 7;
        assert_eq!(sq.output(1), 12);
        sq.phase = 3;
        assert_eq!(sq.output(1), -12);
        // Duty 2 (50%): phases 0, 5, 6, 7 high.
        sq.phase = 5;
        assert_eq!(sq.output(2), 12);
        sq.phase = 4;
        assert_eq!(sq.output(2), -12);
        // Duty 3 (75%): only phases 0 and 7 low.
        sq.phase = 0;
        assert_eq!(sq.output(3), -12);
        sq.phase = 3;
        assert_eq!(sq.output(3), 12);
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
    fn length_retrigger_quirk_consumes_extra_tick() {
        // A trigger landing on an even sequencer step reloads the full
        // length; one landing just before a length step (odd step)
        // consumes an extra tick immediately.
        let mut even = LengthEnvelope::default();
        even.trigger(64, 8, 0, false);
        assert_eq!(even.length, 64);
        let mut odd = LengthEnvelope::default();
        odd.trigger(64, 8, 0, true);
        assert_eq!(odd.length, 63);
        // The wave channel shares the quirk.
        let mut wave = Wave::default();
        wave.trigger(200, false, false);
        assert_eq!(wave.length, 200);
        let mut wave_odd = Wave::default();
        wave_odd.trigger(200, false, true);
        assert_eq!(wave_odd.length, 199);
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
    fn sweep_direction_flip_zombie_kills_channel() {
        // Arm in decrement mode and run one calculation.
        let mut sq = Square::default();
        sq.core.active = true;
        sq.freq_shadow = 0x200;
        sq.write_sweep(0x19); // pace 1, decrement, shift 1
        sq.sweep_timer = 1;
        assert!(sq.tick_sweep());
        // Flipping decrement -> increment kills the voice (zombie rule).
        sq.write_sweep(0x11); // pace 1, increment, shift 1
        assert!(!sq.core.active);
    }

    #[test]
    fn sweep_direction_flip_without_history_spares_channel() {
        // No calculation ran yet: flipping direction is harmless.
        let mut sq = Square::default();
        sq.core.active = true;
        sq.freq_shadow = 0x200;
        sq.write_sweep(0x19); // pace 1, decrement, shift 1
        sq.write_sweep(0x11); // pace 1, increment, shift 1
        assert!(sq.core.active);
        // Same-direction rewrite after a calculation is harmless too.
        sq.sweep_timer = 1;
        assert!(sq.tick_sweep());
        sq.write_sweep(0x11);
        assert!(sq.core.active);
        // Increment -> decrement flips are always safe.
        let mut inc = Square::default();
        inc.core.active = true;
        inc.freq_shadow = 0x200;
        inc.write_sweep(0x11);
        inc.sweep_timer = 1;
        assert!(inc.tick_sweep());
        inc.write_sweep(0x19);
        assert!(inc.core.active);
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

    #[test]
    fn wave_64digit_mode_alternates_banks() {
        let mut w = Wave::default();
        w.trigger(255, true, false);
        assert!(w.active);
        assert_eq!(w.bank, 0);
        // Bank 0 replays 0xFF (nibble 15 -> +7), bank 1 replays 0x00
        // (nibble 0 -> -8): the output must follow the playing bank.
        let mut ram = [0xFFu8; 0x20];
        ram[16..].fill(0x00);
        assert_eq!(w.output(&ram, 1, false), 15);
        // Fastest rate: 8 T-cycles per digit, 32 digits per wrap.
        for _ in 0..32 * 8 {
            w.tick_timer(0x7FF);
        }
        assert_eq!(w.phase, 0);
        assert_eq!(w.bank, 1);
        assert_eq!(w.output(&ram, 1, false), 0);
        for _ in 0..32 * 8 {
            w.tick_timer(0x7FF);
        }
        assert_eq!(w.phase, 0);
        assert_eq!(w.bank, 0);
        assert_eq!(w.output(&ram, 1, false), 15);
    }

    #[test]
    fn validate_accepts_live_ch2_with_zero_sweep_pace() {
        // ch2 has no sweep unit: pace stays at the never-written zero
        // while sounding. This everyday state must import cleanly.
        let live_ch2 = Square {
            core: LengthEnvelope {
                active: true,
                ..Default::default()
            },
            sweep_pace: 0,
            ..Default::default()
        };
        live_ch2.validate(false).unwrap();
        // ch1 (sweep unit present) still rejects a live pace-0 voice
        // once a shift arms the sweep path...
        let armed = Square {
            sweep_shift: 3,
            ..live_ch2
        };
        assert!(armed.validate(true).is_err());
        // ...but accepts the never-written shape (pace 0, no shift).
        live_ch2.validate(true).unwrap();
        // ...and the untouched default either way.
        Square::default().validate(true).unwrap();
        Square::default().validate(false).unwrap();
    }

    #[test]
    fn tick_sweep_treats_pace_zero_and_off_code_as_disabled() {
        for pace in [0u8, 8] {
            let mut sq = Square {
                core: LengthEnvelope {
                    active: true,
                    ..Default::default()
                },
                freq_shadow: 0x7F0,
                sweep_shift: 1,
                sweep_pace: pace,
                sweep_timer: 0,
                ..Default::default()
            };
            // No underflow panic, no overflow kill, no timer movement.
            for _ in 0..300 {
                assert!(sq.tick_sweep());
            }
            assert!(sq.core.active);
            assert_eq!(sq.sweep_timer, 0);
        }
    }

    #[test]
    fn trigger_keeps_phase_and_reloads_timer() {
        // The duty step survives retriggers (only its timer restarts):
        // freq 0x700 reloads 4096 T-cycles, advancing every 4097th tick
        // (the model's steady-state cadence).
        let mut sq = Square::default();
        sq.trigger(0x700, 64, 15, 0xF000, false);
        assert_eq!(sq.phase_for_test(), 0);
        assert_eq!(sq.timer_horizon(), Some(4096));
        for _ in 0..4096 {
            sq.tick_timer(0x700, false, 0, 512);
        }
        assert_eq!(sq.phase_for_test(), 0);
        sq.tick_timer(0x700, false, 0, 512);
        assert_eq!(sq.phase_for_test(), 1);
        // Retrigger keeps step 1 and restarts its full period.
        sq.trigger(0x700, 64, 15, 0xF000, false);
        assert_eq!(sq.phase_for_test(), 1);
        assert_eq!(sq.timer_horizon(), Some(4096));
    }

    #[test]
    fn envelope_extra_tick_adds_one_to_reload() {
        let mut le = LengthEnvelope::default();
        le.trigger(64, 8, 0xF200, false);
        assert_eq!(le.env_timer_for_test(), 2);
        le.envelope_extra_tick();
        assert_eq!(le.env_timer_for_test(), 3);
    }
}
