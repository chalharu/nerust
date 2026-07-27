/// LR35902 CPU register file.
///
/// Flags (F register): bit 7=Z, bit 6=N, bit 5=H, bit 4=C.
/// Bits 3-0 are always 0 on read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuRegisters {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
}

impl CpuRegisters {
    pub fn new() -> Self {
        Self {
            a: 0x01,
            f: 0xB0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
            sp: 0xFFFE,
            pc: 0x0100,
        }
    }

    // --- 16-bit register pairs ---

    pub fn af(&self) -> u16 {
        ((self.a as u16) << 8) | (self.f & 0xF0) as u16
    }

    pub fn set_af(&mut self, v: u16) {
        self.a = (v >> 8) as u8;
        self.f = (v as u8) & 0xF0;
    }

    pub fn bc(&self) -> u16 {
        ((self.b as u16) << 8) | (self.c as u16)
    }

    pub fn set_bc(&mut self, v: u16) {
        self.b = (v >> 8) as u8;
        self.c = v as u8;
    }

    pub fn de(&self) -> u16 {
        ((self.d as u16) << 8) | (self.e as u16)
    }

    pub fn set_de(&mut self, v: u16) {
        self.d = (v >> 8) as u8;
        self.e = v as u8;
    }

    pub fn hl(&self) -> u16 {
        ((self.h as u16) << 8) | (self.l as u16)
    }

    pub fn set_hl(&mut self, v: u16) {
        self.h = (v >> 8) as u8;
        self.l = v as u8;
    }

    // --- flags ---

    pub fn z_flag(&self) -> bool {
        self.f & 0x80 != 0
    }
    pub fn set_z(&mut self, v: bool) {
        self.f = (self.f & !0x80) | if v { 0x80 } else { 0 };
    }
    pub fn n_flag(&self) -> bool {
        self.f & 0x40 != 0
    }
    pub fn set_n(&mut self, v: bool) {
        self.f = (self.f & !0x40) | if v { 0x40 } else { 0 };
    }
    pub fn h_flag(&self) -> bool {
        self.f & 0x20 != 0
    }
    pub fn set_h(&mut self, v: bool) {
        self.f = (self.f & !0x20) | if v { 0x20 } else { 0 };
    }
    pub fn c_flag(&self) -> bool {
        self.f & 0x10 != 0
    }
    pub fn set_c(&mut self, v: bool) {
        self.f = (self.f & !0x10) | if v { 0x10 } else { 0 };
    }
}

impl Default for CpuRegisters {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registers_match_dmg_bios() {
        let r = CpuRegisters::new();
        assert_eq!(r.a, 0x01);
        assert_eq!(r.f, 0xB0);
        assert_eq!(r.c, 0x13);
        assert_eq!(r.e, 0xD8);
        assert_eq!(r.l, 0x4D);
        assert_eq!(r.sp, 0xFFFE);
        assert_eq!(r.pc, 0x0100);
    }

    #[test]
    fn set_af_preserves_lower_nibble_of_f() {
        let mut r = CpuRegisters::new();
        r.set_af(0x1234);
        assert_eq!(r.a, 0x12);
        assert_eq!(r.f, 0x30); // 0x34 & 0xF0
    }

    #[test]
    fn flags_round_trip() {
        let mut r = CpuRegisters::new();
        r.set_z(true);
        r.set_n(true);
        r.set_h(false);
        r.set_c(true);
        assert!(r.z_flag());
        assert!(r.n_flag());
        assert!(!r.h_flag());
        assert!(r.c_flag());
        assert_eq!(r.f, 0xD0); // Z=1 N=1 H=0 C=1
    }

    #[test]
    fn register_pairs_round_trip() {
        let mut r = CpuRegisters::new();
        r.set_bc(0xABCD);
        assert_eq!(r.bc(), 0xABCD);
        r.set_de(0xDEAD);
        assert_eq!(r.de(), 0xDEAD);
        r.set_hl(0xBEEF);
        assert_eq!(r.hl(), 0xBEEF);
    }
}
