/// APU length counter unit.
///
/// Counts down from the loaded value. When it reaches 0 and is enabled,
/// the channel is turned off.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LengthCounter {
    max: u16,
    counter: u16,
    enabled: bool,
}

impl LengthCounter {
    pub fn new(max: u16) -> Self {
        Self {
            max,
            counter: max,
            enabled: false,
        }
    }

    /// Clock the length counter. Returns true if the counter reached 0
    /// and the channel should be turned off.
    pub fn clock(&mut self) -> bool {
        if self.enabled && self.counter > 0 {
            self.counter -= 1;
            self.counter == 0
        } else {
            false
        }
    }

    /// Load the length counter from a register value.
    /// The counter is set to `max - value`, clamped to 0.
    pub fn load(&mut self, value: u8) {
        self.counter = self.max.saturating_sub(u16::from(value));
    }

    /// Reload the counter at zero (used during trigger).
    /// If the counter is 0, reload with max.
    pub fn reload_at_zero(&mut self) {
        if self.counter == 0 {
            self.counter = self.max;
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn counter(&self) -> u16 {
        self.counter
    }

    pub fn max(&self) -> u16 {
        self.max
    }

    pub fn set_counter(&mut self, value: u16) {
        self.counter = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_counter_clocks_down() {
        let mut lc = LengthCounter::new(64);
        lc.set_enabled(true);
        lc.load(60); // counter = 4
        assert_eq!(lc.counter(), 4);
        assert!(!lc.clock()); // 3
        assert!(!lc.clock()); // 2
        assert!(!lc.clock()); // 1
        assert!(lc.clock()); // 0 -> returns true
    }

    #[test]
    fn length_counter_disabled_does_not_count() {
        let mut lc = LengthCounter::new(64);
        lc.set_enabled(false);
        lc.load(60);
        assert!(!lc.clock());
        assert_eq!(lc.counter(), 4);
    }

    #[test]
    fn length_counter_reload_at_zero() {
        let mut lc = LengthCounter::new(64);
        lc.load(64); // counter = 0
        assert_eq!(lc.counter(), 0);
        lc.reload_at_zero();
        assert_eq!(lc.counter(), 64);
    }
}
