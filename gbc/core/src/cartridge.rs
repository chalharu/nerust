use std::time::SystemTime;

use crate::cartridge_mbc::Mbc;

/// Top-level cartridge struct wrapping the ROM data and MBC.
///
/// Delegates ROM/RAM read/write to the MBC trait object.
#[derive(Debug)]
pub struct Cartridge {
    mbc: Box<dyn Mbc>,
    has_rtc: bool,
}

impl Cartridge {
    pub fn new(mbc: Box<dyn Mbc>) -> Self {
        let has_rtc = mbc.has_rtc();
        Self { mbc, has_rtc }
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

    pub fn step_clock(&mut self) {
        if self.has_rtc {
            self.mbc.step_clock();
        }
    }

    pub fn sync_rtc(&mut self, now: SystemTime) {
        if self.has_rtc {
            self.mbc.sync_rtc(now);
        }
    }

    pub fn export_persistent_state(&self, now: SystemTime) -> Result<Option<Vec<u8>>, String> {
        self.mbc.export_persistent_state(now)
    }

    pub fn import_persistent_state(&mut self, data: &[u8]) -> Result<(), String> {
        self.mbc.import_persistent_state(data)
    }
}

impl Default for Cartridge {
    fn default() -> Self {
        Self::new(Box::new(crate::cartridge_mbc::RomOnly::new(vec![
            0;
            0x8000
        ])))
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

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
        let mut cart = Cartridge::new(Box::new(crate::cartridge_mbc::Mbc1::new(
            rom,
            vec![],
            false,
        )));
        cart.write_rom(0x2000, 0x02);
        assert_eq!(cart.read_rom(0x4000), 0xCC);
    }

    #[test]
    fn mbc1_write_ram_enable_delegates() {
        let mut ram = vec![0u8; 0x2000];
        ram[0] = 0x55;
        let mut cart = Cartridge::new(Box::new(crate::cartridge_mbc::Mbc1::new(
            vec![0; 0x8000],
            ram,
            false,
        )));
        assert_eq!(cart.read_ram(0xA000), 0xFF); // disabled
        cart.write_rom(0x0000, 0x0A); // enable
        assert_eq!(cart.read_ram(0xA000), 0x55);
    }

    #[test]
    fn has_battery_delegates_to_mbc() {
        let cart = Cartridge::default();
        assert!(!cart.has_battery());
    }

    #[test]
    fn ram_data_delegates_to_mbc() {
        let cart = Cartridge::new(Box::new(crate::cartridge_mbc::Mbc1::new(
            vec![0; 0x8000],
            vec![0x42; 8],
            true,
        )));
        assert!(cart.ram_data().is_some());
    }

    #[test]
    fn serialize_mbc_state_delegates() {
        let cart = Cartridge::new(Box::new(crate::cartridge_mbc::Mbc1::new(
            vec![0; 0x20000],
            vec![0; 0x2000],
            false,
        )));
        let state = cart.serialize_mbc_state();
        assert_eq!(state.len(), 6);
    }

    #[test]
    fn ram_restore_delegates() {
        let mut cart = Cartridge::new(Box::new(crate::cartridge_mbc::Mbc1::new(
            vec![0; 0x8000],
            vec![0; 0x2000],
            false,
        )));
        let data = vec![0xAA; 0x2000];
        cart.ram_restore(&data);
        // Enable RAM to verify data was restored
        cart.write_rom(0x0000, 0x0A);
        assert_eq!(cart.read_ram(0xA000), 0xAA);
    }

    #[test]
    fn rtc_clock_delegates_for_clocked_cartridge() {
        let mut cart = Cartridge::new(Box::new(crate::cartridge_mbc::Mbc3::new(
            vec![0; 0x8000],
            vec![],
            true,
            true,
        )));
        cart.write_rom(0x0000, 0x0A);
        cart.write_rom(0x4000, 0x08);

        for _ in 0..4_194_304 {
            cart.step_clock();
        }
        cart.write_rom(0x6000, 0);
        cart.write_rom(0x6000, 1);

        assert_eq!(cart.read_ram(0xA000), 1);
        assert!(cart.export_persistent_state(SystemTime::UNIX_EPOCH).is_ok());
    }
}
