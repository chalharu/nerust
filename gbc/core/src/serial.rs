/// Serial (Link Cable) stub.
///
/// v1: no link cable emulation. External device is always disconnected,
/// so reads return 0xFF. Master transfers complete immediately and
/// request the Serial interrupt. Outgoing characters are buffered.
pub struct Serial {
    sb: u8,
    sc: u8,
    /// Characters transmitted via serial (captured for test harness).
    output: Vec<u8>,
}

impl Serial {
    pub fn new() -> Self {
        Self {
            sb: 0,
            sc: 0x7E,
            output: Vec::new(),
        }
    }

    pub fn read_sb(&self) -> u8 {
        0xFF
    }

    pub fn write_sb(&mut self, v: u8) {
        self.sb = v;
    }

    pub fn read_sc(&self) -> u8 {
        self.sc | 0x7E
    }

    /// Returns true if a master transfer just completed (interrupt requested).
    pub fn write_sc(&mut self, v: u8) -> bool {
        self.sc = v & 0x81;
        if self.sc & 0x81 == 0x81 {
            self.sc &= !0x80;
            self.output.push(self.sb);
            return true;
        }
        false
    }

    /// Characters written to the serial port.
    pub fn output(&self) -> &[u8] {
        &self.output
    }
}

impl Default for Serial {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_sb_returns_0xff() {
        let s = Serial::new();
        assert_eq!(s.read_sb(), 0xFF);
    }

    #[test]
    fn read_sc_masks_unused_bits_high() {
        let s = Serial::new();
        assert_eq!(s.read_sc() & 0x7E, 0x7E);
    }

    #[test]
    fn write_sc_master_internal_clears_bit7_and_returns_true() {
        let mut s = Serial::new();
        let completed = s.write_sc(0x81); // master mode + transfer start
        assert!(completed);
        assert_eq!(s.read_sc() & 0x80, 0x00);
    }

    #[test]
    fn write_sc_external_clock_returns_false() {
        let mut s = Serial::new();
        let completed = s.write_sc(0x80); // transfer start only, not master
        assert!(!completed);
        assert_eq!(s.read_sc() & 0x80, 0x80);
    }
}
