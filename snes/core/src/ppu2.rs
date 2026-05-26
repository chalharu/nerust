const CGRAM_LEN: usize = 512;

#[derive(Debug, Clone)]
pub(crate) struct Ppu2 {
    registers: [u8; 0x40],
    cgram: [u8; CGRAM_LEN],
    inidisp: u8,
    cgadd: u8,
    cgram_latch: u8,
    cgram_byte: bool,
}

impl Default for Ppu2 {
    fn default() -> Self {
        Self {
            registers: [0; 0x40],
            cgram: [0; CGRAM_LEN],
            inidisp: 0,
            cgadd: 0,
            cgram_latch: 0,
            cgram_byte: false,
        }
    }
}

impl Ppu2 {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn write(&mut self, offset: u16, value: u8) -> bool {
        match offset {
            0x2100 => {
                self.store_register(offset, value);
                self.inidisp = value;
                true
            }
            0x2121 => {
                self.store_register(offset, value);
                self.cgadd = value;
                self.cgram_byte = false;
                true
            }
            0x2122 => {
                self.store_register(offset, value);
                if self.cgram_byte {
                    let index = (usize::from(self.cgadd) * 2) % CGRAM_LEN;
                    self.cgram[index] = self.cgram_latch;
                    self.cgram[(index + 1) % CGRAM_LEN] = value;
                    self.cgadd = self.cgadd.wrapping_add(1);
                } else {
                    self.cgram_latch = value;
                }
                self.cgram_byte = !self.cgram_byte;
                true
            }
            0x2123..=0x2133 => {
                self.store_register(offset, value);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn read(&mut self, offset: u16) -> Option<u8> {
        match offset {
            0x213B => {
                let index =
                    (usize::from(self.cgadd) * 2 + usize::from(self.cgram_byte)) % CGRAM_LEN;
                let value = self.cgram[index];
                if self.cgram_byte {
                    self.cgadd = self.cgadd.wrapping_add(1);
                }
                self.cgram_byte = !self.cgram_byte;
                Some(value)
            }
            0x213C | 0x213D => Some(0),
            0x213F => Some(0x01),
            _ => None,
        }
    }

    pub(crate) fn peek(&self, offset: u16) -> Option<u8> {
        match offset {
            0x2100 | 0x2121..=0x2133 => Some(self.registers[register_index(offset)]),
            0x213B => {
                let index =
                    (usize::from(self.cgadd) * 2 + usize::from(self.cgram_byte)) % CGRAM_LEN;
                Some(self.cgram[index])
            }
            0x213C | 0x213D => Some(0),
            0x213F => Some(0x01),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn inidisp(&self) -> u8 {
        self.inidisp
    }

    #[cfg(test)]
    pub(crate) fn peek_cgram(&self, index: usize) -> u8 {
        self.cgram[index % CGRAM_LEN]
    }

    fn store_register(&mut self, offset: u16, value: u8) {
        self.registers[register_index(offset)] = value;
    }
}

fn register_index(offset: u16) -> usize {
    usize::from(offset - 0x2100)
}

#[cfg(test)]
mod tests {
    use super::Ppu2;

    #[test]
    fn cgram_data_writes_commit_after_second_byte() {
        let mut ppu2 = Ppu2::new();

        assert!(ppu2.write(0x2121, 0x01));
        assert!(ppu2.write(0x2122, 0x7F));
        assert_eq!(ppu2.peek_cgram(2), 0x00);
        assert!(ppu2.write(0x2122, 0x00));

        assert_eq!(ppu2.inidisp(), 0x00);
        assert_eq!(ppu2.peek_cgram(2), 0x7F);
        assert_eq!(ppu2.peek_cgram(3), 0x00);
    }
}
