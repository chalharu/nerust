use super::{envelope::Envelope, length_counter::LengthCounter, timer::Timer};

/// Divisor lookup table for the noise channel timer.
/// Index 0 is treated as 0.5 (represented as 8 = 0.5 * 16).
const DIVISOR_TABLE: [u16; 8] = [8, 16, 32, 48, 64, 80, 96, 112];

/// CH4: Noise channel.
///
/// Uses a 15-bit LFSR (Linear Feedback Shift Register) to generate
/// pseudo-random noise.
#[derive(Debug, Clone)]
pub(crate) struct Noise {
    pub timer: Timer,
    pub length: LengthCounter,
    pub envelope: Envelope,
    /// 15-bit LFSR state (bit 14 = MSB).
    pub lfsr: u16,
    /// LFSR width mode: false = 15-bit, true = 7-bit.
    pub width_mode: bool,
    /// Clock shift (NR43 bits 7-4).
    pub clock_shift: u8,
    /// Divisor index (NR43 bits 2-0).
    pub divisor_index: u8,
    /// Whether the DAC is enabled.
    pub dac_enabled: bool,
    /// Whether the channel is active (NR52 status).
    pub active: bool,
}

impl Noise {
    pub fn new() -> Self {
        Self {
            timer: Timer::default(),
            length: LengthCounter::new(64),
            envelope: Envelope::new(),
            lfsr: 0x7FFF,
            width_mode: false,
            clock_shift: 0,
            divisor_index: 0,
            dac_enabled: false,
            active: false,
        }
    }

    /// Step the channel timer.
    /// Pan Docs: "Using a noise channel clock shift of 14 or 15 results
    /// in the LFSR receiving no clocks."
    pub fn step(&mut self) {
        if self.timer.step() && self.clock_shift < 14 {
            self.clock_lfsr();
        }
    }

    /// Clock the LFSR (XNOR of bits 0 and 1).
    fn clock_lfsr(&mut self) {
        // XNOR of bit 0 and bit 1 (Pan Docs: 1 if identical, 0 otherwise)
        let xnor_bit = ((self.lfsr & 1) ^ ((self.lfsr >> 1) & 1)) ^ 1;
        // Shift right and put result in bit 14
        self.lfsr = (self.lfsr >> 1) | (xnor_bit << 14);
        // 7-bit mode: also copy to bit 6
        if self.width_mode {
            self.lfsr = (self.lfsr & !0x40) | (xnor_bit << 6);
        }
    }

    /// Get the digital output (0 or volume).
    pub fn output(&self) -> u8 {
        if !self.dac_enabled || !self.active {
            return 0;
        }
        // Output is 0 if LFSR bit 0 is 0, otherwise the envelope volume
        if self.lfsr & 1 == 0 {
            0
        } else {
            self.envelope.output()
        }
    }

    /// Calculate the timer period from NR43.
    /// frequency = 262144 / (divisor * 2^shift)
    /// Divisor 0 is treated as 0.5.
    /// Pan Docs: "shift being equal to 14 or 15 stops the channel from
    /// being clocked entirely."
    fn calculate_timer_period(&self) -> u16 {
        // clock_shift >= 14: LFSR is not clocked, use max period
        if self.clock_shift >= 14 {
            return u16::MAX;
        }
        let divisor = if self.divisor_index == 0 {
            8 // 0.5 * 16
        } else {
            DIVISOR_TABLE[self.divisor_index as usize]
        };
        divisor << self.clock_shift
    }

    /// Handle trigger event.
    pub fn trigger(&mut self, envelope_extra_tick: bool) {
        if self.length.counter() == 0 {
            self.length.reload_at_zero();
            self.length.set_enabled(false);
        }
        if self.dac_enabled && !self.active {
            self.active = true;
        }
        // Reload timer
        self.timer.set_period(self.calculate_timer_period());
        // Reload envelope with extra tick if DIV-APU next step clocks envelope
        self.envelope.reload_timer(envelope_extra_tick);
        // Reset LFSR
        self.lfsr = 0x7FFF;
    }

    /// Handle NR41 write: load length counter.
    pub fn write_nr41(&mut self, value: u8) {
        self.length.load(value & 0x3F);
    }

    /// Handle NR42 write: update volume and DAC.
    pub fn write_nr42(&mut self, value: u8) {
        self.envelope.reload_volume(value);
        self.dac_enabled = value & 0xF8 != 0;
        if !self.dac_enabled {
            self.active = false;
        }
    }

    /// Handle NR43 write: update frequency and randomness.
    pub fn write_nr43(&mut self, value: u8) {
        self.clock_shift = (value >> 4) & 0x0F;
        self.width_mode = value & 0x08 != 0;
        self.divisor_index = value & 0x07;
        // Update timer period
        if self.active {
            self.timer.set_period(self.calculate_timer_period());
        }
    }

    /// Handle NR44 write: trigger, length enable.
    pub fn write_nr44(&mut self, value: u8, next_div_lsb: bool, envelope_extra_tick: bool) {
        // Length enable
        let length_enable = value & 0x40 != 0;

        // Trigger
        if value & 0x80 != 0 {
            self.trigger(envelope_extra_tick);
        }

        // Length glitch
        if length_enable && !self.length.enabled() && next_div_lsb && self.length.counter() > 0 {
            self.length.set_enabled(true);
            if self.length.clock() {
                if value & 0x80 != 0 {
                    self.length.set_counter(self.length.max() - 1);
                } else {
                    self.active = false;
                }
            }
        }

        self.length.set_enabled(length_enable);
    }
}

impl Default for Noise {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_lfsr_xnor() {
        let mut ch = Noise::new();
        ch.lfsr = 0x7FFF; // All bits 1
        ch.clock_lfsr();
        // XNOR of bit 0 (1) and bit 1 (1) = 1
        // After shift: bit 14 = 1, rest shifted right
        // Expected: 0x7FFF >> 1 | (1 << 14) = 0x3FFF | 0x4000 = 0x7FFF
        assert_eq!(ch.lfsr, 0x7FFF);
    }

    #[test]
    fn noise_output_zero_when_lfsr_bit0_zero() {
        let mut ch = Noise::new();
        ch.active = true;
        ch.dac_enabled = true;
        ch.envelope.reload_volume(0xF0);
        ch.envelope.reload_timer(false);
        ch.lfsr = 0x7FFE; // bit 0 = 0
        assert_eq!(ch.output(), 0);
    }

    #[test]
    fn noise_output_volume_when_lfsr_bit0_one() {
        let mut ch = Noise::new();
        ch.active = true;
        ch.dac_enabled = true;
        ch.envelope.reload_volume(0xF0);
        ch.envelope.reload_timer(false);
        ch.lfsr = 0x7FFF; // bit 0 = 1
        assert_eq!(ch.output(), 15);
    }

    #[test]
    fn noise_clock_shift_14_or_15_stops_lfsr() {
        let mut ch = Noise::new();
        ch.active = true;
        ch.dac_enabled = true;
        ch.envelope.reload_volume(0xF0);
        ch.envelope.reload_timer(false);

        // Set clock_shift to 14
        ch.clock_shift = 14;
        ch.timer.set_period(ch.calculate_timer_period());
        assert_eq!(ch.timer.period(), u16::MAX);

        // LFSR should not change when clock_shift >= 14
        let initial_lfsr = ch.lfsr;
        ch.step(); // timer overflow, but LFSR not clocked
        assert_eq!(ch.lfsr, initial_lfsr);
    }
}
