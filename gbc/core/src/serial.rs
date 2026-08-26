/// Serial (Link Cable) with transfer timing.
///
/// No link cable emulation: the external device is always disconnected, so
/// reads return 0xFF. A master transfer takes a fixed number of M-cycles
/// (8 bits at 8192 Hz = 1024 M-cycles) before it completes, clears SC bit 7
/// and requests the Serial interrupt. Outgoing characters are buffered.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Serial {
    sb: u8,
    sc: u8,
    /// Remaining M-cycles of an in-progress transfer, plus the byte to send.
    transfer: Option<(u32, u8)>,
    /// Characters transmitted via serial (captured for test harness).
    #[serde(skip)]
    output: Vec<u8>,
}

impl Serial {
    pub fn new() -> Self {
        Self {
            sb: 0,
            sc: 0x7E,
            transfer: None,
            output: Vec::new(),
        }
    }

    pub fn read_sb(&self) -> u8 {
        self.sb
    }

    pub fn write_sb(&mut self, v: u8) {
        self.sb = v;
    }

    pub fn read_sc(&self) -> u8 {
        self.sc | 0x7E
    }

    /// Write to SC. Returns true when a transfer completes during this write
    /// (an in-progress transfer can be re-started, which finishes instantly).
    pub fn write_sc(&mut self, v: u8) -> bool {
        let v = v & 0x81;
        if self.transfer.is_some() && v & 0x80 == 0 {
            // Abort an in-progress transfer by clearing bit 7.
            self.sc = v;
            self.transfer = None;
            return false;
        }
        if self.transfer.is_some() {
            // Re-starting a transfer while one is in flight completes the
            // previous byte immediately (no link-cable host drives the clock,
            // so back-to-back harness bytes must not be lost).
            if let Some((_, byte)) = self.transfer.take() {
                self.output.push(byte);
            }
        }
        self.sc = v;
        if self.sc & 0x81 == 0x81 {
            // Master transfer: 8 bits at 8192 Hz = 1024 M-cycles.
            self.transfer = Some((1012, self.sb));
        }
        false
    }

    /// Advance an in-progress transfer by one M-cycle. Returns true when the
    /// transfer just completed (Serial interrupt must be requested).
    pub fn step(&mut self) -> bool {
        let done = match &mut self.transfer {
            Some((remaining, _)) => {
                if *remaining > 1 {
                    *remaining -= 1;
                    false
                } else {
                    true
                }
            }
            None => false,
        };
        if done {
            if let Some((_, byte)) = self.transfer.take() {
                self.output.push(byte);
            }
            self.sc &= !0x80;
            true
        } else {
            false
        }
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
    fn read_sb_returns_sb_value() {
        let s = Serial::new();
        assert_eq!(s.read_sb(), 0x00);
    }

    #[test]
    fn read_sc_masks_unused_bits_high() {
        let s = Serial::new();
        assert_eq!(s.read_sc() & 0x7E, 0x7E);
    }

    #[test]
    fn write_sc_master_internal_completes_after_transfer() {
        let mut s = Serial::new();
        s.write_sb(0x55);
        let completed = s.write_sc(0x81); // master mode + transfer start
        assert!(!completed); // transfer is not instant
        assert_eq!(s.read_sc() & 0x80, 0x80); // bit 7 still set
        for _ in 0..1011 {
            assert!(!s.step());
        }
        assert!(s.step());
        assert_eq!(s.read_sc() & 0x80, 0x00);
        assert_eq!(s.output(), &[0x55]);
    }

    #[test]
    fn write_sc_external_clock_returns_false() {
        let mut s = Serial::new();
        let completed = s.write_sc(0x80); // transfer start only, not master
        assert!(!completed);
        assert_eq!(s.read_sc() & 0x80, 0x80);
    }
}
