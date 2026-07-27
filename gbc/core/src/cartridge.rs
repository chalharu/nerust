use crate::cartridge_mbc::Mbc;

/// Top-level cartridge struct wrapping the ROM data and MBC.
///
/// Delegates ROM/RAM read/write to the MBC trait object.
#[derive(Debug)]
pub struct Cartridge {
    mbc: Box<dyn Mbc>,
}

impl Cartridge {
    pub fn new(mbc: Box<dyn Mbc>) -> Self {
        Self { mbc }
    }

    pub fn read_rom(&self, addr: u16) -> u8 {
        if addr < 0x4000 {
            self.mbc.read_rom0(addr)
        } else {
            self.mbc.read_rom_n(addr)
        }
    }

    pub fn read_ram(&self, addr: u16) -> u8 {
        self.mbc.read_ram(addr)
    }

    pub fn write_rom(&mut self, addr: u16, value: u8) {
        self.mbc.write_rom(addr, value);
    }

    pub fn write_ram(&mut self, addr: u16, value: u8) {
        self.mbc.write_ram(addr, value);
    }

    pub fn has_battery(&self) -> bool {
        self.mbc.has_battery()
    }

    pub fn ram_data(&self) -> Option<&[u8]> {
        self.mbc.ram_data()
    }

    pub fn ram_restore(&mut self, data: &[u8]) {
        self.mbc.ram_restore(data);
    }

    pub fn serialize_mbc_state(&self) -> Vec<u8> {
        self.mbc.serialize_state()
    }

    pub fn deserialize_mbc_state(&mut self, data: &[u8]) -> Result<(), String> {
        self.mbc.deserialize_state(data)
    }
}

impl Default for Cartridge {
    fn default() -> Self {
        Self::new(Box::new(crate::cartridge_mbc::RomOnly::new(vec![0; 0x8000])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cartridge_reads_zero_rom() {
        let cart = Cartridge::default();
        assert_eq!(cart.read_rom(0x0000), 0x00);
    }

    #[test]
    fn cartridge_delegates_to_mbc() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x42] = 0xAB;
        let cart = Cartridge::new(Box::new(crate::cartridge_mbc::RomOnly::new(rom)));
        assert_eq!(cart.read_rom(0x0042), 0xAB);
    }

    #[test]
    fn mbc1_write_rom_delegates() {
        let mut rom = vec![0u8; 0x20000];
        rom[0x8000] = 0xCC;
        let mut cart = Cartridge::new(Box::new(crate::cartridge_mbc::Mbc1::new(rom, vec![], false)));
        cart.write_rom(0x2000, 0x02);
        assert_eq!(cart.read_rom(0x4000), 0xCC);
    }

    #[test]
    fn mbc1_write_ram_enable_delegates() {
        let mut ram = vec![0u8; 0x2000];
        ram[0] = 0x55;
        let mut cart = Cartridge::new(Box::new(crate::cartridge_mbc::Mbc1::new(vec![0; 0x8000], ram, false)));
        assert_eq!(cart.read_ram(0xA000), 0xFF); // disabled
        cart.write_rom(0x0000, 0x0A); // enable
        assert_eq!(cart.read_ram(0xA000), 0x55);
    }
}
