use super::{envelope::Envelope, length_counter::LengthCounter, timer::Timer};

/// Duty cycle lookup table.
const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [1, 0, 0, 0, 0, 0, 0, 1], // 25%
    [1, 0, 0, 0, 0, 1, 1, 1], // 50%
    [0, 1, 1, 1, 1, 1, 1, 0], // 75%
];

/// CH2: Pulse channel (no sweep).
#[derive(Debug, Clone)]
pub(crate) struct Square2 {
    pub timer: Timer,
    pub length: LengthCounter,
    pub envelope: Envelope,
    /// Duty cycle mode (0-3).
    pub duty: u8,
    /// Current position in the duty waveform (0-7).
    pub duty_pos: u8,
    /// Whether the DAC is enabled.
    pub dac_enabled: bool,
    /// Whether the channel is active (NR52 status).
    pub active: bool,
    /// 11-bit frequency value (NR23 | (NR24 & 7) << 8).
    pub frequency: u16,
}

impl Square2 {
    pub fn new() -> Self {
        Self {
            timer: Timer::default(),
            length: LengthCounter::new(64),
            envelope: Envelope::new(),
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

    /// Check if the DAC is enabled based on NR22.
    pub fn update_dac(&mut self, nr22: u8) {
        self.dac_enabled = nr22 & 0xF8 != 0;
        if !self.dac_enabled {
            self.active = false;
        }
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
        // Reload frequency from registers
        self.timer.set_counter(self.timer.period());
        // Reload envelope with extra tick if DIV-APU next step clocks envelope
        self.envelope.reload_timer(envelope_extra_tick);
    }

    /// Handle NR21 write: update duty and length.
    pub fn write_nr21(&mut self, value: u8) {
        self.duty = (value >> 6) & 3;
        self.length.load(value & 0x3F);
    }

    /// Handle NR22 write: update volume and DAC.
    pub fn write_nr22(&mut self, value: u8) {
        self.envelope.reload_volume(value);
        self.update_dac(value);
    }

    /// Handle NR23 write: update frequency low byte.
    pub fn write_nr23(&mut self, value: u8) {
        self.frequency = (self.frequency & 0x700) | value as u16;
        self.timer.set_period(2048u16.wrapping_sub(self.frequency));
    }

    /// Handle NR24 write: update frequency high bits, trigger, length enable.
    pub fn write_nr24(&mut self, value: u8, next_div_lsb: bool, envelope_extra_tick: bool) {
        let was_active = self.active;

        // Update frequency high bits
        self.frequency = (self.frequency & 0xFF) | ((value as u16 & 0x07) << 8);
        self.timer.set_period(2048u16.wrapping_sub(self.frequency));

        // Length enable
        let length_enable = value & 0x40 != 0;

        // Trigger
        if value & 0x80 != 0 {
            self.trigger(envelope_extra_tick);
        }

        // Length glitch
        if length_enable && !self.length.enabled() && next_div_lsb && self.length.counter() > 0 {
            self.length.clock();
            if self.length.counter() == 0 && !was_active {
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

impl Default for Square2 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square2_duty_output() {
        let mut ch = Square2::new();
        ch.active = true;
        ch.dac_enabled = true;
        ch.duty = 2; // 50%
        ch.envelope.reload_volume(0xF0); // volume 15
        ch.envelope.reload_timer(false);

        // 50% duty: 1,0,0,0,0,1,1,1
        let mut outputs = Vec::new();
        for _ in 0..8 {
            outputs.push(ch.output());
            ch.duty_pos = (ch.duty_pos + 1) & 7;
        }
        assert_eq!(outputs, vec![15, 0, 0, 0, 0, 15, 15, 15]);
    }
}
