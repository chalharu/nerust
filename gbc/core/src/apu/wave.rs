use super::{length_counter::LengthCounter, timer::Timer};

/// CH3: Wave channel.
///
/// Plays arbitrary waveforms from 16 bytes of Wave RAM (32 x 4-bit samples).
/// The sample rate is determined by the frequency register.
#[derive(Debug, Clone)]
pub(crate) struct Wave {
    pub timer: Timer,
    pub length: LengthCounter,
    /// Wave RAM: 16 bytes = 32 x 4-bit samples.
    pub wave_ram: [u8; 16],
    /// Current sample index (0-31).
    pub position: u8,
    /// Output level shift from NR32 bits 6-5:
    /// 00 = mute, 01 = 100%, 10 = 50%, 11 = 25%
    pub volume_shift: u8,
    /// Whether the DAC is enabled (NR30 bit 7).
    pub dac_enabled: bool,
    /// Whether the channel is active (NR52 status).
    pub active: bool,
    /// 11-bit frequency value.
    pub frequency: u16,
    /// Sample buffer: holds the last sample read.
    /// NOT cleared on trigger, only when APU is turned off.
    pub sample_buffer: u8,
}

impl Wave {
    pub fn new() -> Self {
        Self {
            timer: Timer::default(),
            length: LengthCounter::new(256),
            wave_ram: [0; 16],
            position: 0,
            volume_shift: 0,
            dac_enabled: false,
            active: false,
            frequency: 0,
            sample_buffer: 0,
        }
    }

    /// Step the channel timer. Called at 2,097,152 Hz (master/2).
    pub fn step(&mut self) {
        if self.timer.step() {
            // Read the sample at current position
            self.sample_buffer = self.read_sample(self.position);
            // Advance to next position
            self.position = (self.position + 1) & 31;
        }
    }

    /// Get the digital output (0-15).
    pub fn output(&self) -> u8 {
        if !self.dac_enabled {
            return 0;
        }
        // Output the buffered sample, shifted by volume control
        self.sample_buffer >> self.volume_shift
    }

    /// Read a 4-bit sample from Wave RAM.
    /// Index 0-31, upper nibble first.
    fn read_sample(&self, index: u8) -> u8 {
        let byte = self.wave_ram[index as usize / 2];
        if index & 1 == 0 {
            byte >> 4
        } else {
            byte & 0x0F
        }
    }

    /// Handle trigger event.
    pub fn trigger(&mut self) {
        if self.length.counter() == 0 {
            self.length.reload_at_zero();
            self.length.set_enabled(false);
        }
        if self.dac_enabled && !self.active {
            self.active = true;
        }
        // Reload frequency
        self.timer.set_counter(self.timer.period());
        // Position is reset to 0
        self.position = 0;
        // sample_buffer is NOT cleared (Pan Docs: "does not clear nor refresh this buffer")
        // It will be cleared only when APU is turned off
    }

    /// Clear the sample buffer (called when APU is turned off).
    pub fn clear_buffer(&mut self) {
        self.sample_buffer = 0;
    }

    /// Handle NR30 write: update DAC.
    pub fn write_nr30(&mut self, value: u8) {
        self.dac_enabled = value & 0x80 != 0;
        if !self.dac_enabled {
            self.active = false;
        }
    }

    /// Handle NR31 write: load length counter.
    pub fn write_nr31(&mut self, value: u8) {
        self.length.load(value);
    }

    /// Handle NR32 write: update output level.
    pub fn write_nr32(&mut self, value: u8) {
        self.volume_shift = (value >> 5) & 3;
    }

    /// Handle NR33 write: update frequency low byte.
    pub fn write_nr33(&mut self, value: u8) {
        self.frequency = (self.frequency & 0x700) | value as u16;
        self.timer.set_period(2048u16.wrapping_sub(self.frequency));
    }

    /// Handle NR34 write: update frequency high bits, trigger, length enable.
    pub fn write_nr34(&mut self, value: u8) {
        let was_active = self.active;

        // Update frequency high bits
        self.frequency = (self.frequency & 0xFF) | ((value as u16 & 0x07) << 8);
        self.timer.set_period(2048u16.wrapping_sub(self.frequency));

        // Length enable
        let length_enable = value & 0x40 != 0;

        // Trigger
        if value & 0x80 != 0 {
            self.trigger();
        }

        // Length glitch
        if length_enable && !self.length.enabled() && self.length.counter() > 0 {
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

    /// Read from Wave RAM.
    pub fn read_wave_ram(&self, addr: u16) -> u8 {
        self.wave_ram[(addr - 0xFF30) as usize]
    }

    /// Write to Wave RAM.
    pub fn write_wave_ram(&mut self, addr: u16, value: u8) {
        self.wave_ram[(addr - 0xFF30) as usize] = value;
    }
}

impl Default for Wave {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_read_sample_upper_nibble() {
        let mut ch = Wave::new();
        ch.wave_ram[0] = 0xAB;
        // Index 0 = upper nibble of byte 0 = 0xA
        assert_eq!(ch.read_sample(0), 0x0A);
    }

    #[test]
    fn wave_read_sample_lower_nibble() {
        let mut ch = Wave::new();
        ch.wave_ram[0] = 0xAB;
        // Index 1 = lower nibble of byte 0 = 0xB
        assert_eq!(ch.read_sample(1), 0x0B);
    }

    #[test]
    fn wave_output_muted_when_dac_off() {
        let ch = Wave::new();
        assert_eq!(ch.output(), 0);
    }

    #[test]
    fn wave_output_volume_shift() {
        let mut ch = Wave::new();
        ch.dac_enabled = true;
        ch.sample_buffer = 0x0F;
        ch.volume_shift = 0; // 100%
        assert_eq!(ch.output(), 0x0F);

        ch.volume_shift = 1; // 50%
        assert_eq!(ch.output(), 0x07);

        ch.volume_shift = 2; // 25%
        assert_eq!(ch.output(), 0x03);
    }
}
