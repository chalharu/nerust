/// APU timer unit.
///
/// Counts down from `period` to 0, then reloads and signals a tick.
/// Used by all channels to clock their respective circuits.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Timer {
    period: u16,
    counter: u16,
}

impl Timer {
    pub fn step(&mut self) -> bool {
        if self.counter == 0 {
            self.counter = self.period;
            true
        } else {
            self.counter -= 1;
            false
        }
    }

    pub fn period(&self) -> u16 {
        self.period
    }

    pub fn set_period(&mut self, period: u16) {
        self.period = period;
    }

    pub fn counter(&self) -> u16 {
        self.counter
    }

    pub fn set_counter(&mut self, counter: u16) {
        self.counter = counter;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_counts_down_and_reloads() {
        let mut timer = Timer {
            period: 3,
            counter: 3,
        };
        assert!(!timer.step()); // 2
        assert!(!timer.step()); // 1
        assert!(!timer.step()); // 0
        assert!(timer.step()); // reload to 3, return true
        assert_eq!(timer.counter(), 3);
    }

    #[test]
    fn timer_zero_period_always_ticks() {
        let mut timer = Timer {
            period: 0,
            counter: 0,
        };
        assert!(timer.step());
        assert!(timer.step());
    }
}
