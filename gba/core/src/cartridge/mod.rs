pub mod header;
pub mod save;

use self::header::GbaHeader;
use self::save::helpers::read_slice;
use self::save::{SaveBackend, SaveType, create_save_backend, detect_save_type};

#[derive(Debug)]
pub struct Cartridge {
    pub header: GbaHeader,
    pub rom: Vec<u8>,
    pub save: Box<dyn SaveBackend>,
}

impl Cartridge {
    pub fn new(rom: Vec<u8>) -> Option<Self> {
        let header = GbaHeader::parse(&rom)?;
        let save_type = detect_save_type(&rom);
        let save = create_save_backend(save_type);
        Some(Self { header, rom, save })
    }

    pub fn read_rom(&self, addr: u32, width: u8) -> u32 {
        let len = self.rom.len();
        if len == 0 {
            return 0xFFFFFFFF;
        }
        let base = 0x08000000;
        let raw_off = ((addr - base) & 0x01FF_FFFF) as usize;
        let off = if len.is_power_of_two() {
            raw_off & (len - 1)
        } else {
            raw_off % len
        };
        read_slice(&self.rom, off, width)
    }

    pub fn read_sram(&self, addr: u32, width: u8) -> u32 {
        self.save.read(addr, width)
    }

    pub fn write_sram(&mut self, addr: u32, width: u8, value: u32) {
        self.save.write(addr, width, value);
    }

    pub fn save_type(&self) -> SaveType {
        self.save.save_type()
    }

    pub fn has_battery(&self) -> bool {
        self.save.has_battery()
    }

    pub fn ram_data(&self) -> Option<&[u8]> {
        self.save.ram_data()
    }

    pub fn ram_restore(&mut self, data: &[u8]) {
        self.save.ram_restore(data);
    }
}

#[cfg(test)]
mod tests {
    use super::header::finalize_test_gba_rom;
    use super::*;

    fn make_rom_with_save(marker: &[u8]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x1000];
        finalize_test_gba_rom(&mut rom);
        let off = 0x200;
        rom[off..off + marker.len()].copy_from_slice(marker);
        rom
    }

    #[test]
    fn rom_3mirrors_same() {
        let mut rom = vec![0u8; 0x8000];
        finalize_test_gba_rom(&mut rom);
        for (i, byte) in rom.iter_mut().enumerate() {
            *byte = (i & 0xFF) as u8;
        }
        let cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.read_rom(0x08000000, 4), cart.read_rom(0x0A000000, 4));
        assert_eq!(cart.read_rom(0x08000000, 4), cart.read_rom(0x0C000000, 4));
    }

    #[test]
    fn non_power_of_two_rom_3mirrors_same() {
        let mut rom = vec![0u8; 0x1001];
        finalize_test_gba_rom(&mut rom);
        rom[0x1000] = 0xA5;
        let cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.read_rom(0x08001000, 1), 0xA5);
        assert_eq!(cart.read_rom(0x0A001000, 1), 0xA5);
        assert_eq!(cart.read_rom(0x0C001000, 1), 0xA5);
    }

    #[test]
    fn sram_rw() {
        let rom = make_rom_with_save(b"SRAM_V100");
        let mut cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.save_type(), SaveType::Sram);
        cart.write_sram(0x0E000000, 1, 0x42);
        assert_eq!(cart.read_sram(0x0E000000, 1), 0x42);
    }

    #[test]
    fn detect_flash128_priority() {
        let rom = make_rom_with_save(b"FLASH1M_V102");
        let cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.save_type(), SaveType::Flash128);
    }

    #[test]
    fn flash_cmd_aa_55_90() {
        let rom = make_rom_with_save(b"FLASH_V130");
        let mut cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.save_type(), SaveType::Flash64);
        cart.write_sram(0x0E005555, 1, 0xAA);
        cart.write_sram(0x0E002AAA, 1, 0x55);
        cart.write_sram(0x0E005555, 1, 0x90);
        // ID mode should be active
        let manuf = cart.read_sram(0x0E000000, 1);
        assert_eq!(manuf, 0x32);
    }
}
