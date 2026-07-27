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
