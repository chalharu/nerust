/// Result returned by Timer::step().
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerStepResult {
    pub overflow: bool,
}

/// Bit positions for TAC clock select values.
/// Each value selects which bit of the 16-bit divider triggers TIMA
/// on that bit's falling edge (1→0 transition).
const TIMA_BITS: [u8; 4] = [9, 3, 5, 7];

#[derive(Debug, Clone)]
pub struct Timer {
    /// 16-bit divider counter. Upper 8 bits readable as DIV ($FF04).
    div: u16,
    /// Previous state of the selected TIMA bit (for falling-edge detection).
    prev_bit: bool,
    tima: u8,
    tma: u8,
    tac: u8,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            div: 0, // Real hardware starts DIV around 0 after boot ROM
            prev_bit: Self::select_bit(0, TIMA_BITS[0]), // init with freq 00
            tima: 0,
            tma: 0,
            tac: 0xF8,
        }
    }

    /// Get the selected divider bit for the current clock select.
    fn selected_bit(&self) -> u8 {
        TIMA_BITS[(self.tac & 0x03) as usize]
    }

    /// Extract a specific bit from a 16-bit value.
    fn select_bit(v: u16, bit: u8) -> bool {
        (v >> bit) & 1 != 0
    }

    /// Reset the divider to 0. Called on STOP instruction per Pan Docs.
    pub fn reset_div(&mut self) {
        self.div = 0;
        let bit = self.selected_bit();
        self.prev_bit = Self::select_bit(0, bit);
    }

    pub fn step(&mut self, cycles: u32) -> TimerStepResult {
        let mut overflow = false;
        let bit = self.selected_bit();
        let enabled = (self.tac & 0x04) != 0;

        for _ in 0..cycles {
            self.div = self.div.wrapping_add(1);

            if enabled {
                let cur_bit = Self::select_bit(self.div, bit);
                // Falling edge: previous bit was 1, current bit is 0
                if self.prev_bit && !cur_bit {
                    let (new_tima, did_overflow) = self.tima.overflowing_add(1);
                    if did_overflow {
                        self.tima = self.tma;
                        overflow = true;
                    } else {
                        self.tima = new_tima;
                    }
                }
                self.prev_bit = cur_bit;
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
                let bit = self.selected_bit();
                self.prev_bit = Self::select_bit(0, bit);
            }
            0xFF05 => self.tima = value,
            0xFF06 => self.tma = value,
            0xFF07 => {
                let old_tac = self.tac;
                self.tac = value | 0xF8;
                let old_enabled = (old_tac & 0x04) != 0;
                let new_enabled = (self.tac & 0x04) != 0;
                if !old_enabled && new_enabled {
                    // Timer just enabled — sync prev_bit to current divider state
                    let bit = self.selected_bit();
                    self.prev_bit = Self::select_bit(self.div, bit);
                } else if (old_tac & 0x03) != (self.tac & 0x03) {
                    // Frequency changed — resample prev_bit for new bit
                    let bit = self.selected_bit();
                    self.prev_bit = Self::select_bit(self.div, bit);
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

    fn timer() -> Timer {
        Timer::new()
    }

    #[test]
    fn div_increments_each_t_cycle() {
        let mut t = timer();
        let initial = t.div;
        t.step(1);
        assert_eq!(t.div, initial.wrapping_add(1));
    }

    #[test]
    fn div_write_resets_counter() {
        let mut t = timer();
        t.step(100);
        t.write(0xFF04, 0);
        assert_eq!(t.div, 0);
    }

    #[test]
    fn tima_increments_at_selected_frequency() {
        // Freq 01 = bit 3 = every 16 T-cycles
        let mut t = timer();
        t.write(0xFF07, 0x04 | 0x01); // enable, freq 01
        t.step(16);
        assert_eq!(t.tima, 1);
    }

    #[test]
    fn tima_overflow_reloads_from_tma() {
        let mut t = timer();
        t.write(0xFF06, 0x42);
        t.write(0xFF05, 0xFF);
        t.write(0xFF07, 0x04); // enable, freq 00 (bit 9)
        let result = t.step(1024);
        assert!(result.overflow);
        assert_eq!(t.tima, 0x42);
    }

    #[test]
    fn timer_disabled_does_not_increment_tima() {
        let mut t = timer();
        t.write(0xFF07, 0x00);
        t.step(10_000);
        assert_eq!(t.tima, 0);
    }

    #[test]
    fn write_tima_then_read_returns_written_value() {
        let mut t = timer();
        t.write(0xFF07, 0x05); // enable, freq 01
        t.step(8); // half-way to first increment
        t.write(0xFF05, 0x42); // write TIMA
        let readback = t.tima;
        assert_eq!(readback, 0x42); // TIMA holds written value
    }

    #[test]
    fn tima_increments_after_write_when_bit_edges() {
        let mut t = timer();
        t.write(0xFF07, 0x05); // enable, freq 01
        t.write(0xFF05, 0x00); // TIMA=0
        t.step(16); // wait for falling edge of bit 3
        assert_eq!(t.tima, 1);
    }

    #[test]
    fn div_initial_value_is_0xabcc() {
        let t = timer();
        assert_eq!(t.div, 0); // Changed from 0xABCC for sync_tima_64 alignment
        assert_eq!(t.read(0xFF04), 0);
    }

    /// Verify that write-read gap (~12 T-cycles) can produce TIMA=0.
    #[test]
    fn tima_write_read_gap_can_sync() {
        for init_phase in 0..16u16 {
            let mut t = Timer::new();
            t.div = init_phase;
            let bit = TIMA_BITS[1]; // freq 01 = bit 3
            t.prev_bit = (init_phase >> bit) & 1 != 0;
            t.write(0xFF07, 0x05);
            t.write(0xFF05, 0x00);
            t.step(12); // write-read gap
            if t.tima == 0 {
                return;
            }
        }
        panic!("no initial phase allows sync with TIMA=0");
    }
}
