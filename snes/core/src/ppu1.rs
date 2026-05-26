const VRAM_LEN: usize = 64 * 1024;
const OAM_LEN: usize = 544;

#[derive(Debug, Clone)]
pub(crate) struct Ppu1 {
    registers: [u8; 0x40],
    vram: [u8; VRAM_LEN],
    oam: [u8; OAM_LEN],
    vmain: u8,
    vmadd: u16,
    oam_byte_addr: u16,
}

impl Default for Ppu1 {
    fn default() -> Self {
        Self {
            registers: [0; 0x40],
            vram: [0; VRAM_LEN],
            oam: [0; OAM_LEN],
            vmain: 0,
            vmadd: 0,
            oam_byte_addr: 0,
        }
    }
}

impl Ppu1 {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn write(&mut self, offset: u16, value: u8) -> bool {
        match offset {
            0x2102 => {
                self.store_register(offset, value);
                self.oam_byte_addr = (self.oam_byte_addr & 0x0200) | (u16::from(value) << 1);
                true
            }
            0x2103 => {
                self.store_register(offset, value);
                self.oam_byte_addr = (self.oam_byte_addr & 0x01FE) | (u16::from(value & 0x01) << 9);
                true
            }
            0x2104 => {
                self.store_register(offset, value);
                self.oam[usize::from(self.oam_byte_addr % OAM_LEN as u16)] = value;
                self.oam_byte_addr = (self.oam_byte_addr + 1) % OAM_LEN as u16;
                true
            }
            0x2115 => {
                self.store_register(offset, value);
                self.vmain = value;
                true
            }
            0x2116 => {
                self.store_register(offset, value);
                self.vmadd = (self.vmadd & 0xFF00) | u16::from(value);
                true
            }
            0x2117 => {
                self.store_register(offset, value);
                self.vmadd = (self.vmadd & 0x00FF) | (u16::from(value) << 8);
                true
            }
            0x2118 => {
                self.store_register(offset, value);
                self.write_vram_byte(false, value);
                true
            }
            0x2119 => {
                self.store_register(offset, value);
                self.write_vram_byte(true, value);
                true
            }
            0x2101 | 0x2105..=0x2114 | 0x211A..=0x2120 => {
                self.store_register(offset, value);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn read(&mut self, offset: u16) -> Option<u8> {
        match offset {
            0x2134..=0x2136 => Some(0),
            0x2138 => {
                let value = self.oam[usize::from(self.oam_byte_addr % OAM_LEN as u16)];
                self.oam_byte_addr = (self.oam_byte_addr + 1) % OAM_LEN as u16;
                Some(value)
            }
            0x2139 => Some(self.read_vram_byte(false, true)),
            0x213A => Some(self.read_vram_byte(true, true)),
            0x213E => Some(0x01),
            _ => None,
        }
    }

    pub(crate) fn peek(&self, offset: u16) -> Option<u8> {
        match offset {
            0x2101..=0x2120 => Some(self.registers[register_index(offset)]),
            0x2134..=0x2136 => Some(0),
            0x2138 => Some(self.oam[usize::from(self.oam_byte_addr % OAM_LEN as u16)]),
            0x2139 => Some(self.read_vram_peek(false)),
            0x213A => Some(self.read_vram_peek(true)),
            0x213E => Some(0x01),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn peek_vram(&self, address: usize) -> u8 {
        self.vram[address % VRAM_LEN]
    }

    #[cfg(test)]
    pub(crate) fn vmadd(&self) -> u16 {
        self.vmadd
    }

    fn store_register(&mut self, offset: u16, value: u8) {
        self.registers[register_index(offset)] = value;
    }

    fn write_vram_byte(&mut self, high: bool, value: u8) {
        let byte_index = self.vram_byte_index(high);
        self.vram[byte_index] = value;
        if self.should_increment_after(high) {
            self.vmadd = self.vmadd.wrapping_add(vram_increment_words(self.vmain));
        }
    }

    fn read_vram_byte(&mut self, high: bool, advance: bool) -> u8 {
        let value = self.vram[self.vram_byte_index(high)];
        if advance && self.should_increment_after(high) {
            self.vmadd = self.vmadd.wrapping_add(vram_increment_words(self.vmain));
        }
        value
    }

    fn read_vram_peek(&self, high: bool) -> u8 {
        self.vram[self.vram_byte_index(high)]
    }

    fn vram_byte_index(&self, high: bool) -> usize {
        let remapped = remap_vmadd(self.vmadd, self.vmain);
        (usize::from(remapped) * 2 + usize::from(high)) % VRAM_LEN
    }

    fn should_increment_after(&self, high: bool) -> bool {
        let increment_after_high = self.vmain & 0x80 != 0;
        if increment_after_high { high } else { !high }
    }
}

fn register_index(offset: u16) -> usize {
    usize::from(offset - 0x2100)
}

fn remap_vmadd(address: u16, vmain: u8) -> u16 {
    match (vmain >> 2) & 0x03 {
        0 => address,
        1 => {
            let rem = address & 0x00FF;
            (address & 0xFF00) | ((rem << 3) & 0x00FF) | (rem >> 5)
        }
        2 => {
            let rem = address & 0x01FF;
            (address & 0xFE00) | ((rem << 3) & 0x01FF) | (rem >> 6)
        }
        3 => {
            let rem = address & 0x03FF;
            (address & 0xFC00) | ((rem << 3) & 0x03FF) | (rem >> 7)
        }
        _ => unreachable!(),
    }
}

fn vram_increment_words(vmain: u8) -> u16 {
    match vmain & 0x03 {
        0 => 1,
        1 => 32,
        _ => 128,
    }
}

#[cfg(test)]
mod tests {
    use super::Ppu1;

    #[test]
    fn vram_data_writes_follow_vmain_increment_mode() {
        let mut ppu1 = Ppu1::new();

        assert!(ppu1.write(0x2115, 0x80));
        assert!(ppu1.write(0x2116, 0x00));
        assert!(ppu1.write(0x2117, 0x00));
        assert!(ppu1.write(0x2118, 0x34));
        assert!(ppu1.write(0x2119, 0x12));

        assert_eq!(ppu1.peek_vram(0), 0x34);
        assert_eq!(ppu1.peek_vram(1), 0x12);
        assert_eq!(ppu1.vmadd(), 1);
    }

    #[test]
    fn vram_remap_mode1_rotates_lower_8_bits() {
        let mut ppu1 = Ppu1::new();

        assert!(ppu1.write(0x2115, 0x84));
        assert!(ppu1.write(0x2116, 0xE5));
        assert!(ppu1.write(0x2117, 0x12));
        assert!(ppu1.write(0x2118, 0x34));
        assert!(ppu1.write(0x2119, 0x12));

        assert_eq!(ppu1.peek_vram(0x122F * 2), 0x34);
        assert_eq!(ppu1.peek_vram(0x122F * 2 + 1), 0x12);
        assert_eq!(ppu1.vmadd(), 0x12E6);
    }

    #[test]
    fn vram_remap_mode2_rotates_lower_9_bits() {
        let mut ppu1 = Ppu1::new();

        assert!(ppu1.write(0x2115, 0x88));
        assert!(ppu1.write(0x2116, 0xE5));
        assert!(ppu1.write(0x2117, 0x23));
        assert!(ppu1.write(0x2118, 0x78));
        assert!(ppu1.write(0x2119, 0x56));

        assert_eq!(ppu1.peek_vram(0x232F * 2), 0x78);
        assert_eq!(ppu1.peek_vram(0x232F * 2 + 1), 0x56);
        assert_eq!(ppu1.vmadd(), 0x23E6);
    }

    #[test]
    fn vram_remap_mode3_rotates_lower_10_bits() {
        let mut ppu1 = Ppu1::new();

        assert!(ppu1.write(0x2115, 0x8C));
        assert!(ppu1.write(0x2116, 0xE5));
        assert!(ppu1.write(0x2117, 0x43));
        assert!(ppu1.write(0x2118, 0xBC));
        assert!(ppu1.write(0x2119, 0x9A));

        assert_eq!(ppu1.peek_vram(0x432F * 2), 0xBC);
        assert_eq!(ppu1.peek_vram(0x432F * 2 + 1), 0x9A);
        assert_eq!(ppu1.vmadd(), 0x43E6);
    }
}
