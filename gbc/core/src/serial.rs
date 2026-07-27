/// Serial (Link Cable) stub.
///
/// v1: no link cable emulation. External device is always disconnected,
/// so reads return 0xFF and transfer completions clear the SC master flag.
pub struct Serial {
    sb: u8,
    sc: u8,
}

impl Serial {
    pub fn new() -> Self {
        Self { sb: 0, sc: 0x7E }
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

    pub fn write_sc(&mut self, v: u8) {
        self.sc = v & 0x81;
        if self.sc & 0x81 == 0x81 {
            self.sc &= !0x80;
        }
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
    fn write_sc_master_internal_clears_bit7() {
        let mut s = Serial::new();
        s.write_sc(0x81); // master mode + transfer start
        assert_eq!(s.read_sc() & 0x80, 0x00);
    }

    #[test]
    fn write_sc_external_clock_does_not_clear_bit7() {
        let mut s = Serial::new();
        s.write_sc(0x80); // transfer start only, not master
        assert_eq!(s.read_sc() & 0x80, 0x80);
    }
}
