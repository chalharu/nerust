/// APU volume envelope unit.
///
/// Ticks at 64 Hz. Period=0 is treated as 8 (Obscure Behavior).
/// When the timer expires, the volume is increased or decreased
/// depending on the add_mode flag.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Envelope {
    /// Current output volume (0-15).
    volume: u8,
    /// Envelope period (1-7; 0 is treated as 8 internally).
    period: u8,
    /// Current timer countdown.
    timer: u8,
    /// Whether the envelope is enabled.
    enabled: bool,
    /// Direction: true = increase, false = decrease.
    add_mode: bool,
    /// Initial volume loaded from NRx2 bits 7-4.
    initial_volume: u8,
}

impl Envelope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clock the envelope at 64 Hz.
    pub fn clock(&mut self) {
        if !self.enabled || self.period == 0 {
            return;
        }
        if self.timer > 0 {
            self.timer -= 1;
        } else {
            self.timer = self.period;
            if self.add_mode && self.volume < 15 {
                self.volume += 1;
            } else if !self.add_mode && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }

    /// Reload the envelope timer. Used during trigger.
    /// Obscure Behavior: if DIV-APU next step would clock the envelope,
    /// the timer is reloaded with period + 1.
    pub fn reload_timer(&mut self, extra: bool) {
        self.timer = if extra {
            // Add 1 to the period when the envelope would be clocked
            self.period.wrapping_add(1)
        } else {
            self.period
        };
    }

    /// Reload the volume from NRx2 initial volume.
    pub fn reload_volume(&mut self, nr2: u8) {
        self.initial_volume = (nr2 >> 4) & 0x0F;
        self.volume = self.initial_volume;
        self.add_mode = nr2 & 0x08 != 0;
        self.period = nr2 & 0x07;
        self.enabled = nr2 & 0xF8 != 0;
    }

    pub fn output(&self) -> u8 {
        self.volume
    }

    pub fn period(&self) -> u8 {
        self.period
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn volume(&self) -> u8 {
        self.volume
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_increases_volume() {
        let mut env = Envelope::default();
        env.reload_volume(0x8B); // volume=8, increase (bit3=1), period=3
        env.reload_timer(false);
        assert_eq!(env.output(), 8);

        env.clock(); // timer 3 -> 2
        env.clock(); // timer 2 -> 1
        env.clock(); // timer 1 -> 0
        env.clock(); // timer 0 -> reload to 3, volume 8+1=9
        assert_eq!(env.output(), 9);
    }

    #[test]
    fn envelope_decreases_volume() {
        let mut env = Envelope::default();
        env.reload_volume(0x03); // volume=0, decrease (bit3=0), period=3
        env.enabled = true; // manually enable (period=0 disables via reload_volume)
        env.reload_timer(false);
        env.volume = 8; // set to 8 manually
        assert_eq!(env.output(), 8);

        env.clock(); // timer 3 -> 2
        env.clock(); // timer 2 -> 1
        env.clock(); // timer 1 -> 0
        env.clock(); // timer 0 -> reload to 3, volume 8-1=7
        assert_eq!(env.output(), 7);
    }

    #[test]
    fn envelope_period_zero_does_not_tick() {
        let mut env = Envelope::default();
        env.reload_volume(0x80); // volume=8, period=0
        env.reload_timer(false);
        assert_eq!(env.volume, 8);
        env.clock(); // should not change
        env.clock();
        env.clock();
        assert_eq!(env.volume, 8); // still 8
    }
}
