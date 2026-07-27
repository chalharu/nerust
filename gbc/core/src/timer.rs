/// Result returned by Timer::step().
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerStepResult {
    pub overflow: bool,
}

const DIV_FREQUENCY: [u16; 4] = [1024, 16, 64, 256];

#[derive(Debug, Clone)]
pub struct Timer {
    div: u16,
    /// Internal clock counter (incremented each T-cycle, reset on overflow or TAC change)
    counter: u16,
    tima: u8,
    tma: u8,
    tac: u8,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            div: 0xABCC,
            counter: 0,
            tima: 0,
            tma: 0,
            tac: 0xF8,
        }
    }

    pub fn step(&mut self, cycles: u32) -> TimerStepResult {
        let mut overflow = false;

        for _ in 0..cycles {
            self.div = self.div.wrapping_add(1);

            let enabled = (self.tac & 0x04) != 0;
            if enabled {
                let threshold = DIV_FREQUENCY[(self.tac & 0x03) as usize];
                self.counter += 1;
                if self.counter >= threshold {
                    self.counter = 0;
                    let (new_tima, did_overflow) = self.tima.overflowing_add(1);
                    if did_overflow {
                        self.tima = self.tma;
                        overflow = true;
                    } else {
                        self.tima = new_tima;
                    }
                }
            }
        }

        TimerStepResult { overflow }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF04 => (self.div >> 8) as u8,
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            0xFF07 => self.tac | 0xF8,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF04 => {
                self.div = 0;
                self.counter = 0;
            }
            0xFF05 => self.tima = value,
            0xFF06 => self.tma = value,
            0xFF07 => {
                let old_enabled = (self.tac & 0x04) != 0;
                self.tac = value | 0xF8;
                let new_enabled = (self.tac & 0x04) != 0;
                if !old_enabled && new_enabled {
                    self.counter = 0;
                }
            }
            _ => {}
        }
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn div_increments_each_t_cycle() {
        let mut timer = Timer::new();
        let initial = timer.div;
        timer.step(1);
        assert_eq!(timer.div, initial.wrapping_add(1));
    }

    #[test]
    fn div_write_resets_counter() {
        let mut timer = Timer::new();
        timer.step(100);
        timer.write(0xFF04, 0);
        assert_eq!(timer.div, 0);
    }

    #[test]
    fn timer_increment_with_tac_enabled() {
        let mut timer = Timer::new();
        timer.write(0xFF07, 0x04); // TAC: enable, freq 00 (4096 Hz = div / 1024)
        let step_count = 1024;
        timer.step(step_count);
        assert_eq!(timer.tima, 1);
    }

    #[test]
    fn timer_overflow_reloads_from_tma() {
        let mut timer = Timer::new();
        timer.write(0xFF06, 0x42);
        timer.write(0xFF05, 0xFF);
        timer.write(0xFF07, 0x04);

        let result = timer.step(1024);
        assert!(result.overflow);
        assert_eq!(timer.tima, 0x42);
    }

    #[test]
    fn timer_disabled_does_not_increment_tima() {
        let mut timer = Timer::new();
        timer.write(0xFF07, 0x00); // TAC: disabled
        timer.step(10_000);
        assert_eq!(timer.tima, 0);
    }
}
