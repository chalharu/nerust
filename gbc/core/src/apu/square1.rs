use super::{envelope::Envelope, length_counter::LengthCounter, timer::Timer};

/// Duty cycle lookup table.
/// Each entry is 8 samples representing the waveform.
const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [1, 0, 0, 0, 0, 0, 0, 1], // 25%
    [1, 0, 0, 0, 0, 1, 1, 1], // 50%
    [0, 1, 1, 1, 1, 1, 1, 0], // 75%
];

/// CH1 sweep state.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Sweep {
    /// Shadow frequency register.
    shadow: u16,
    /// Sweep period countdown.
    countdown: u8,
    /// Current sweep period from NR10.
    period: u8,
    /// Step shift amount.
    shift: u8,
    /// Negate mode (subtraction).
    negate: bool,
    /// Whether the negate mode was used since last trigger.
    negate_used: bool,
    /// Whether the sweep unit is enabled.
    enabled: bool,
}

impl Sweep {
    pub fn new() -> Self {
        Self::default()
    }

    /// Perform frequency calculation: new_freq = freq ± (freq >> shift)
    fn calculate(&self, freq: u16) -> (u16, u16) {
        let delta = freq >> self.shift;
        let new_freq = if self.negate {
            freq.wrapping_sub(delta)
        } else {
            freq.wrapping_add(delta)
        };
        (new_freq, delta)
    }

    /// Check if the frequency would overflow (> 0x7FF).
    fn would_overflow(&self, freq: u16, delta: u16) -> bool {
        if self.negate {
            freq < delta
        } else {
            freq + delta > 0x7FF
        }
    }
}

/// CH1: Pulse channel with frequency sweep.
#[derive(Debug, Clone)]
pub(crate) struct Square1 {
    pub timer: Timer,
    pub length: LengthCounter,
    pub envelope: Envelope,
    pub sweep: Sweep,
    /// Duty cycle mode (0-3).
    pub duty: u8,
    /// Current position in the duty waveform (0-7).
    pub duty_pos: u8,
    /// Whether the DAC is enabled.
    pub dac_enabled: bool,
    /// Whether the channel is active (NR52 status).
    pub active: bool,
    /// 11-bit frequency value (NR13 | (NR14 & 7) << 8).
    pub frequency: u16,
}

impl Square1 {
    pub fn new() -> Self {
        Self {
            timer: Timer::default(),
            length: LengthCounter::new(64),
            envelope: Envelope::new(),
            sweep: Sweep::new(),
            duty: 0,
            duty_pos: 0,
            dac_enabled: false,
            active: false,
            frequency: 0,
        }
    }

    /// Step the channel timer. Called at 1,048,576 Hz (master/4).
    pub fn step(&mut self) {
        if self.timer.step() {
            self.duty_pos = (self.duty_pos + 1) & 7;
        }
    }

    /// Get the digital output (0-15).
    pub fn output(&self) -> u8 {
        if !self.dac_enabled || !self.active {
            return 0;
        }
        if DUTY_TABLE[self.duty as usize][self.duty_pos as usize] == 0 {
            return 0;
        }
        self.envelope.output()
    }

    /// Check if the DAC is enabled based on NR12.
    pub fn update_dac(&mut self, nr12: u8) {
        self.dac_enabled = nr12 & 0xF8 != 0;
        if !self.dac_enabled {
            self.active = false;
        }
    }

    /// Handle trigger event.
    /// Pan Docs: If a channel is triggered when the DIV-APU next step
    /// will clock the volume envelope, the envelope's timer is reloaded
    /// with one greater than it would have been.
    pub fn trigger(&mut self, envelope_extra_tick: bool) {
        if self.length.counter() == 0 {
            self.length.reload_at_zero();
            self.length.set_enabled(false);
        }
        if self.dac_enabled && !self.active {
            self.active = true;
        }
        // Reload frequency from registers
        self.timer.set_counter(self.timer.period());
        // Reload envelope with extra tick if DIV-APU next step clocks envelope
        self.envelope.reload_timer(envelope_extra_tick);
        // Reload sweep
        self.sweep.shadow = self.frequency;
        self.sweep.period &= 7;
        self.sweep.countdown = self.sweep.period ^ 7;
        self.sweep.enabled = self.sweep.period != 0 || self.sweep.shift != 0;
        self.sweep.negate_used = false;
        // Overflow check on trigger if shift != 0
        if self.sweep.shift != 0 {
            let (_new_freq, delta) = self.sweep.calculate(self.frequency);
            self.sweep.negate_used = self.sweep.negate;
            if self.sweep.would_overflow(self.frequency, delta) {
                self.active = false;
            }
        }
    }

    /// Clock the frequency sweep at 128 Hz.
    /// Returns the new frequency if it changed, or None if channel was disabled.
    pub fn clock_sweep(&mut self) -> Option<u16> {
        if !self.sweep.enabled {
            return None;
        }

        self.sweep.countdown = (self.sweep.countdown + 1) & 7;
        if self.sweep.countdown != 7 {
            return None;
        }

        if self.sweep.period == 0 {
            return None;
        }

        self.sweep.negate_used |= self.sweep.negate;
        let (new_freq, delta) = self.sweep.calculate(self.sweep.shadow);

        if self.sweep.shift == 0 {
            // No shift: just check overflow
            if self.sweep.would_overflow(self.sweep.shadow, delta) {
                self.active = false;
                return None;
            }
        } else {
            if self.sweep.would_overflow(self.sweep.shadow, delta) {
                self.active = false;
                return None;
            }
            // Update frequency
            self.sweep.shadow = new_freq;
            self.frequency = new_freq;
            self.timer.set_period(2048u16.wrapping_sub(new_freq));
            // Second overflow check
            let (_, delta2) = self.sweep.calculate(new_freq);
            if self.sweep.would_overflow(new_freq, delta2) {
                self.active = false;
                return None;
            }
        }

        // Reset countdown
        self.sweep.countdown = self.sweep.period ^ 7;

        Some(new_freq)
    }

    /// Handle NR10 write: update sweep configuration.
    pub fn write_nr10(&mut self, value: u8) {
        self.sweep.period = (value >> 4) & 7;
        self.sweep.negate = value & 0x08 != 0;
        self.sweep.shift = value & 7;

        // If negate was used and now cleared, disable channel
        if self.sweep.negate_used && !self.sweep.negate {
            self.active = false;
        }
    }

    /// Handle NR11 write: update duty and length.
    pub fn write_nr11(&mut self, value: u8) {
        self.duty = (value >> 6) & 3;
        self.length.load(value & 0x3F);
    }

    /// Handle NR12 write: update volume and DAC.
    pub fn write_nr12(&mut self, value: u8) {
        self.envelope.reload_volume(value);
        self.update_dac(value);
    }

    /// Handle NR13 write: update frequency low byte.
    pub fn write_nr13(&mut self, value: u8) {
        self.frequency = (self.frequency & 0x700) | value as u16;
        self.timer.set_period(2048u16.wrapping_sub(self.frequency));
    }

    /// Handle NR14 write: update frequency high bits, trigger, length enable.
    /// Pan Docs: Length glitch occurs when writing to NRx4 when the
    /// DIV-APU next step is one that doesn't clock the length timer.
    pub fn write_nr14(&mut self, value: u8, next_div_lsb: bool, envelope_extra_tick: bool) {
        // Update frequency high bits
        self.frequency = (self.frequency & 0xFF) | ((value as u16 & 0x07) << 8);
        self.timer.set_period(2048u16.wrapping_sub(self.frequency));

        // Length enable
        let length_enable = value & 0x40 != 0;

        // Trigger
        if value & 0x80 != 0 {
            self.trigger(envelope_extra_tick);
        }

        // Length glitch: extra clocking when enabling length
        // The glitch fires when:
        // 1. Length is being enabled (length_enable && !previously_enabled)
        // 2. The DIV-APU next step won't clock length (next_div_lsb == true)
        // 3. Length counter is non-zero
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

impl Default for Square1 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square1_duty_output() {
        let mut ch = Square1::new();
        ch.active = true;
        ch.dac_enabled = true;
        ch.duty = 2; // 50%
        ch.envelope.reload_volume(0xF0); // volume 15
        ch.envelope.reload_timer(false);

        // Check duty cycle pattern
        // 50% duty: 1,0,0,0,0,1,1,1
        let mut outputs = Vec::new();
        for _ in 0..8 {
            outputs.push(ch.output());
            ch.duty_pos = (ch.duty_pos + 1) & 7;
        }
        assert_eq!(outputs, vec![15, 0, 0, 0, 0, 15, 15, 15]);
    }
}
