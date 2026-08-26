use super::{Mbc, MbcKind};

/// MBC5 (up to 8 MiB ROM and 128 KiB RAM).
#[derive(Debug, Clone)]
pub struct Mbc5 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    ram_enabled: bool,
    rom_bank: u16,
    ram_bank: u8,
    battery: bool,
}

impl Mbc5 {
    pub fn new(rom: Vec<u8>, ram: Vec<u8>, battery: bool) -> Self {
        Self {
            rom,
            ram,
            ram_enabled: false,
            rom_bank: 1,
            ram_bank: 0,
            battery,
        }
    }
}

impl Mbc for Mbc5 {
    fn kind(&self) -> MbcKind {
        MbcKind::Mbc5
    }

    fn read_rom0(&self, addr: u16) -> u8 {
        self.rom.get(addr as usize).copied().unwrap_or(0xFF)
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        let bank_count = self.rom.len() / 0x4000;
        let bank = if bank_count > 0 {
            (self.rom_bank as usize) & (bank_count - 1)
        } else {
            0
        };
        let offset = bank * 0x4000 + (addr as usize - 0x4000);
        self.rom.get(offset).copied().unwrap_or(0xFF)
    }

    fn write_rom(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => {
                self.ram_enabled = (value & 0x0F) == 0x0A;
            }
            0x2000..=0x2FFF => {
                self.rom_bank = (self.rom_bank & 0x100) | value as u16;
            }
            0x3000..=0x3FFF => {
                self.rom_bank = (self.rom_bank & 0xFF) | ((value as u16 & 0x01) << 8);
            }
            0x4000..=0x5FFF => {
                self.ram_bank = value & 0x0F;
            }
            _ => {}
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        if !self.ram_enabled || self.ram.is_empty() {
            return 0xFF;
        }
        let offset = self.ram_bank as usize * 0x2000 + (addr as usize - 0xA000);
        self.ram.get(offset).copied().unwrap_or(0xFF)
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram_enabled || self.ram.is_empty() {
            return;
        }
        let offset = self.ram_bank as usize * 0x2000 + (addr as usize - 0xA000);
        if let Some(cell) = self.ram.get_mut(offset) {
            *cell = value;
        }
    }

    fn has_battery(&self) -> bool {
        self.battery
    }

    fn ram_data(&self) -> Option<&[u8]> {
        if self.ram.is_empty() {
            None
        } else {
            Some(&self.ram)
        }
    }

    fn ram_restore(&mut self, data: &[u8]) {
        if data.len() <= self.ram.len() {
            self.ram[..data.len()].copy_from_slice(data);
        }
    }

    fn serialize_state(&self) -> Vec<u8> {
        vec![
            self.ram_enabled as u8,
            self.rom_bank as u8,
            (self.rom_bank >> 8) as u8,
            self.ram_bank,
        ]
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() < 4 {
            return Err("MBC5 state too short".into());
        }
        self.ram_enabled = data[0] != 0;
        self.rom_bank = data[1] as u16 | ((data[2] as u16) << 8);
        self.ram_bank = data[3] & 0x0F;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_reads_bank_0_and_1() {
        let mut rom = vec![0u8; 0x80000]; // 128 banks → 512 KiB
        rom[0x0000] = 0xAA;
        rom[0x4000] = 0xBB;
        let mbc = Mbc5::new(rom, vec![0; 0x2000], false);
        assert_eq!(mbc.read_rom0(0x0000), 0xAA);
        assert_eq!(mbc.read_rom_n(0x4000), 0xBB);
    }

    #[test]
    fn low_bank_register_switches_bank() {
        let mut rom = vec![0u8; 0x80000]; // 128 banks
        rom[5 * 0x4000] = 0xCC;
        let mut mbc = Mbc5::new(rom, vec![0; 0x2000], false);
        mbc.write_rom(0x2000, 0x05);
        assert_eq!(mbc.read_rom_n(0x4000), 0xCC);
    }

    #[test]
    fn high_bank_bit_toggles_above_256() {
        let mut rom = vec![0u8; 0x800000]; // 512 banks → 8 MiB
        rom[0x100 * 0x4000] = 0xDD;
        let mut mbc = Mbc5::new(rom, vec![0; 0x2000], false);
        mbc.write_rom(0x2000, 0x00);
        mbc.write_rom(0x3000, 0x01);
        assert_eq!(mbc.read_rom_n(0x4000), 0xDD);
    }

    #[test]
    fn ram_read_requires_enable() {
        let mut ram = vec![0u8; 0x2000];
        ram[0] = 0x77;
        let mut mbc = Mbc5::new(vec![0; 0x8000], ram, false);
        assert_eq!(mbc.read_ram(0xA000), 0xFF);
        mbc.write_rom(0x0000, 0x0A);
        assert_eq!(mbc.read_ram(0xA000), 0x77);
    }
}
