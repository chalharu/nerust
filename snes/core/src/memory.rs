const WRAM_LEN: usize = 128 * 1024;
const WMADD_MASK: u32 = 0x01_FFFF;

#[derive(Debug, Clone)]
pub(crate) struct Memory {
    wram: [u8; WRAM_LEN],
    wmadd: u32,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            wram: [0; WRAM_LEN],
            wmadd: 0,
        }
    }
}

impl Memory {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn read_cpu_bus(&mut self, bank: u8, offset: u16) -> Option<u8> {
        match (bank, offset) {
            (0x7E..=0x7F, _) => Some(self.wram[wram_index(bank, offset)]),
            (0x00..=0x3F | 0x80..=0xBF, 0x0000..=0x1FFF) => Some(self.wram[usize::from(offset)]),
            _ => None,
        }
    }

    pub(crate) fn peek_cpu_bus(&self, bank: u8, offset: u16) -> Option<u8> {
        match (bank, offset) {
            (0x7E..=0x7F, _) => Some(self.wram[wram_index(bank, offset)]),
            (0x00..=0x3F | 0x80..=0xBF, 0x0000..=0x1FFF) => Some(self.wram[usize::from(offset)]),
            _ => None,
        }
    }

    pub(crate) fn write_cpu_bus(&mut self, bank: u8, offset: u16, value: u8) -> bool {
        match (bank, offset) {
            (0x7E..=0x7F, _) => {
                self.wram[wram_index(bank, offset)] = value;
                true
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x0000..=0x1FFF) => {
                self.wram[usize::from(offset)] = value;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn read_mmio(&mut self, offset: u16) -> Option<u8> {
        match offset {
            0x2180 => {
                let value = self.wram[self.wmadd as usize];
                self.wmadd = wrap_wmadd(self.wmadd + 1);
                Some(value)
            }
            _ => None,
        }
    }

    pub(crate) fn peek_mmio(&self, offset: u16) -> Option<u8> {
        match offset {
            0x2180 => Some(self.wram[self.wmadd as usize]),
            0x2181 => Some(self.wmadd as u8),
            0x2182 => Some((self.wmadd >> 8) as u8),
            0x2183 => Some(((self.wmadd >> 16) as u8) & 0x01),
            _ => None,
        }
    }

    pub(crate) fn write_mmio(&mut self, offset: u16, value: u8) -> bool {
        match offset {
            0x2180 => {
                self.wram[self.wmadd as usize] = value;
                self.wmadd = wrap_wmadd(self.wmadd + 1);
                true
            }
            0x2181 => {
                self.wmadd = (self.wmadd & !0x0000FF) | u32::from(value);
                true
            }
            0x2182 => {
                self.wmadd = (self.wmadd & !0x00FF00) | (u32::from(value) << 8);
                true
            }
            0x2183 => {
                self.wmadd = (self.wmadd & !0x010000) | (u32::from(value & 0x01) << 16);
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn peek_wram(&self, address: usize) -> u8 {
        self.wram[address % WRAM_LEN]
    }

    #[cfg(test)]
    pub(crate) fn wmadd(&self) -> u32 {
        self.wmadd
    }
}

fn wrap_wmadd(value: u32) -> u32 {
    value & WMADD_MASK
}

fn wram_index(bank: u8, offset: u16) -> usize {
    (usize::from(bank - 0x7E) << 16) | usize::from(offset)
}

#[cfg(test)]
mod tests {
    use super::Memory;

    #[test]
    fn wram_ports_read_write_and_auto_increment() {
        let mut memory = Memory::new();

        assert!(memory.write_mmio(0x2181, 0x00));
        assert!(memory.write_mmio(0x2182, 0x00));
        assert!(memory.write_mmio(0x2183, 0x00));
        assert!(memory.write_mmio(0x2180, 0x5A));
        assert!(memory.write_mmio(0x2180, 0xC3));

        assert_eq!(memory.peek_wram(0), 0x5A);
        assert_eq!(memory.peek_wram(1), 0xC3);
        assert_eq!(memory.wmadd(), 2);

        assert!(memory.write_mmio(0x2181, 0x00));
        assert!(memory.write_mmio(0x2182, 0x00));
        assert!(memory.write_mmio(0x2183, 0x00));
        assert_eq!(memory.read_mmio(0x2180), Some(0x5A));
        assert_eq!(memory.read_mmio(0x2180), Some(0xC3));
    }
}
