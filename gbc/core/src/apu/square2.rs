use super::{channel, envelope::Envelope, length_counter::LengthCounter, timer::Timer};

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
        channel::step_pulse(&mut self.timer, &mut self.duty_pos);
    }

    /// Get the digital output (0-15).
    pub fn output(&self) -> u8 {
        channel::pulse_output(
            &self.envelope,
            self.duty,
            self.duty_pos,
            self.dac_enabled,
            self.active,
        )
    }

    /// Handle trigger event.
    pub fn trigger(&mut self, envelope_extra_tick: bool) {
        channel::prepare_trigger(&mut self.length, self.dac_enabled, &mut self.active);
        // Reload frequency from registers
        self.timer.set_counter(self.timer.period());
        // Reload envelope with extra tick if DIV-APU next step clocks envelope
        self.envelope.reload_timer(envelope_extra_tick);
    }

    /// Handle NR21 write: update duty and length.
    pub fn write_nr21(&mut self, value: u8) {
        channel::write_pulse_duty_length(value, &mut self.duty, &mut self.length);
    }

    /// Handle NR22 write: update volume and DAC.
    pub fn write_nr22(&mut self, value: u8) {
        channel::write_envelope(
            value,
            &mut self.envelope,
            &mut self.dac_enabled,
            &mut self.active,
        );
    }

    /// Handle NR23 write: update frequency low byte.
    pub fn write_nr23(&mut self, value: u8) {
        channel::write_frequency_low(value, &mut self.frequency, &mut self.timer);
    }

    /// Handle NR24 write: update frequency high bits, trigger, length enable.
    pub fn write_nr24(&mut self, value: u8, next_div_lsb: bool, envelope_extra_tick: bool) {
        channel::write_frequency_high(value, &mut self.frequency, &mut self.timer);

        // Trigger
        if value & 0x80 != 0 {
            self.trigger(envelope_extra_tick);
        }

        channel::apply_length_control(value, next_div_lsb, &mut self.length, &mut self.active);
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
