use crate::cartridge_header::CartridgeHeader;

/// Memory Bank Controller trait.
///
/// Handles ROM bank switching, RAM access, and battery-backed save data.
/// Default implementations are no-ops so ROM Only only needs to implement
/// `read_rom0`, `read_rom_n`, `serialize_state`, and `deserialize_state`.
#[allow(unused_variables)]
pub trait Mbc: std::fmt::Debug {
    fn read_rom0(&self, addr: u16) -> u8;
    fn read_rom_n(&self, addr: u16) -> u8;

    fn write_rom(&mut self, addr: u16, value: u8) {}

    fn read_ram(&self, addr: u16) -> u8 {
        0xFF
    }
    fn write_ram(&mut self, addr: u16, value: u8) {}

    fn has_battery(&self) -> bool {
        false
    }
    fn ram_data(&self) -> Option<&[u8]> {
        None
    }
    fn ram_restore(&mut self, data: &[u8]) {}

    fn serialize_state(&self) -> Vec<u8>;
    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String>;
}

/// ROM Only MBC: no banking, no RAM.
#[derive(Debug, Clone)]
pub struct RomOnly {
    rom: Vec<u8>,
}

impl RomOnly {
    pub fn new(rom: Vec<u8>) -> Self {
        Self { rom }
    }
}

impl Mbc for RomOnly {
    fn read_rom0(&self, addr: u16) -> u8 {
        self.rom[addr as usize]
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        self.rom[addr as usize]
    }

    fn serialize_state(&self) -> Vec<u8> {
        Vec::new()
    }

    fn deserialize_state(&mut self, _data: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

/// MBC1 (max 2 MiB ROM and/or 32 KiB RAM).
#[derive(Debug, Clone)]
pub struct Mbc1 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    ram_enabled: bool,
    rom_bank: u8,
    ram_bank: u8,
    banking_mode: bool,
    battery: bool,
    rom_bank_mask: u8,
}

impl Mbc1 {
    pub fn new(rom: Vec<u8>, ram: Vec<u8>, battery: bool) -> Self {
        let rom_bank_mask = Self::bank_mask(rom.len() / 0x4000);
        Self {
            rom,
            ram,
            ram_enabled: false,
            rom_bank: 1,
            ram_bank: 0,
            banking_mode: false,
            battery,
            rom_bank_mask,
        }
    }

    fn bank_mask(bank_count: usize) -> u8 {
        bank_count.saturating_sub(1) as u8
    }

    fn rom_bank_effective(&self) -> usize {
        let bank = if self.rom_bank == 0 { 1 } else { self.rom_bank };
        let bank_count = self.rom.len() / 0x4000;
        let upper = if bank_count > 32 {
            (self.ram_bank as usize) << 5
        } else {
            0
        };
        (upper | (bank as usize)) & self.rom_bank_mask as usize
    }
}

impl Mbc for Mbc1 {
    fn read_rom0(&self, addr: u16) -> u8 {
        let bank = if self.banking_mode {
            (self.ram_bank as usize) << 5
        } else {
            0
        };
        let offset = bank * 0x4000 + addr as usize;
        self.rom.get(offset).copied().unwrap_or(0xFF)
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        let offset = self.rom_bank_effective() * 0x4000 + (addr as usize - 0x4000);
        self.rom.get(offset).copied().unwrap_or(0xFF)
    }

    fn write_rom(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => {
                self.ram_enabled = (value & 0x0F) == 0x0A;
            }
            0x2000..=0x3FFF => {
                self.rom_bank = value & 0x1F;
            }
            0x4000..=0x5FFF => {
                self.ram_bank = value & 0x03;
            }
            0x6000..=0x7FFF => {
                self.banking_mode = (value & 0x01) != 0;
            }
            _ => {}
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        if !self.ram_enabled || self.ram.is_empty() {
            return 0xFF;
        }
        let bank = if self.banking_mode {
            self.ram_bank as usize
        } else {
            0
        };
        let offset = bank * 0x2000 + (addr as usize - 0xA000);
        self.ram.get(offset).copied().unwrap_or(0xFF)
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram_enabled || self.ram.is_empty() {
            return;
        }
        let bank = if self.banking_mode {
            self.ram_bank as usize
        } else {
            0
        };
        let offset = bank * 0x2000 + (addr as usize - 0xA000);
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
            self.rom_bank,
            self.ram_bank,
            self.banking_mode as u8,
            u8::try_from(self.ram.len()).unwrap_or(0),
            0,
        ]
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() < 6 {
            return Err("MBC1 state too short".into());
        }
        self.ram_enabled = data[0] != 0;
        self.rom_bank = data[1];
        self.ram_bank = data[2] & 0x03;
        self.banking_mode = data[3] != 0;
        Ok(())
    }
}

/// Factory function to create the appropriate MBC from header + ROM data.
pub fn create_mbc(header: &CartridgeHeader, rom: Vec<u8>, ram: Option<Vec<u8>>) -> Box<dyn Mbc> {
    match header.cartridge_type {
        crate::cartridge_header::CartridgeType::RomOnly => Box::new(RomOnly::new(rom)),
        crate::cartridge_header::CartridgeType::Mbc1
        | crate::cartridge_header::CartridgeType::Mbc1Ram
        | crate::cartridge_header::CartridgeType::Mbc1RamBattery => {
            let ram = ram.unwrap_or_else(|| vec![0; header.ram_size.bytes]);
            Box::new(Mbc1::new(rom, ram, header.cartridge_type.has_battery()))
        }
        _ => Box::new(RomOnly::new(rom)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_only_reads_all_addresses_from_rom() {
        let rom = vec![0x42u8; 0x8000];
        let mbc = RomOnly::new(rom);
        assert_eq!(mbc.read_rom0(0x0000), 0x42);
        assert_eq!(mbc.read_rom_n(0x4000), 0x42);
    }

    #[test]
    fn rom_only_has_no_battery_by_default() {
        let mbc = RomOnly::new(vec![0; 0x8000]);
        assert!(!mbc.has_battery());
    }

    #[test]
    fn mbc1_default_reads_bank_0_and_1() {
        let mut rom = vec![0u8; 0x20000]; // 8 banks → 128 KiB
        rom[0x0000] = 0xAA;
        rom[0x4000] = 0xBB;
        let mbc = Mbc1::new(rom, vec![0; 0x2000], false);
        assert_eq!(mbc.read_rom0(0x0000), 0xAA);
        assert_eq!(mbc.read_rom_n(0x4000), 0xBB);
    }

    #[test]
    fn mbc1_bank_switch_reads_correct_bank() {
        let mut rom = vec![0u8; 0x20000]; // 8 banks
        rom[0x8000] = 0xCC; // bank 2, offset 0
        let mut mbc = Mbc1::new(rom, vec![0; 0x2000], false);
        mbc.write_rom(0x2000, 0x02); // select bank 2
        assert_eq!(mbc.read_rom_n(0x4000), 0xCC);
    }

    #[test]
    fn mbc1_ram_read_requires_enable() {
        let mut ram = vec![0u8; 0x2000];
        ram[0] = 0x77;
        let mut mbc = Mbc1::new(vec![0; 0x8000], ram, false);
        assert_eq!(mbc.read_ram(0xA000), 0xFF); // disabled
        mbc.write_rom(0x0000, 0x0A); // enable RAM
        assert_eq!(mbc.read_ram(0xA000), 0x77);
    }

    #[test]
    fn mbc1_bank_0_treated_as_1() {
        let mut rom = vec![0u8; 0x10000]; // 4 banks
        rom[0x0000] = 0xAA;
        rom[0x4000] = 0x11;
        let mut mbc = Mbc1::new(rom, vec![], false);
        mbc.write_rom(0x2000, 0x00); // select bank 0 → treated as 1
        assert_eq!(mbc.read_rom_n(0x4000), 0x11);
    }

    #[test]
    fn mbc1_large_rom_uses_secondary_bank_register() {
        // 2 MiB ROM: 128 banks, uses 2-bit secondary register for bits 5-6
        let mut rom = vec![0u8; 0x200000]; // 128 banks = 2 MiB
        let target_bank = 33; // bank 33 = 32 + 1 → secondary=1, primary=1
        rom[target_bank * 0x4000] = 0xCC;
        let mut mbc = Mbc1::new(rom, vec![], false);
        mbc.write_rom(0x4000, 0x01); // secondary bank = 1
        mbc.write_rom(0x2000, 0x01); // primary bank = 1
        // Effective = (1 << 5) | 1 = 33
        assert_eq!(mbc.read_rom_n(0x4000), 0xCC);
    }

    #[test]
    fn mbc1_mode_1_maps_rom0_to_other_bank() {
        let mut rom = vec![0u8; 0x200000]; // 128 banks (2 MiB)
        let bank32 = 32 * 0x4000;
        rom[bank32] = 0xDD;
        let mut mbc = Mbc1::new(rom, vec![], false);
        mbc.write_rom(0x4000, 0x01); // secondary = 1
        mbc.write_rom(0x6000, 0x01); // mode 1
        assert_eq!(mbc.read_rom0(0x0000), 0xDD); // 0000 reads from bank $20
    }

    #[test]
    fn mbc1_mode_1_allows_ram_banking() {
        let mut ram = vec![0u8; 0x8000]; // 32 KiB (4 banks)
        ram[0] = 0x11;
        ram[0x2000] = 0x22;
        let mut mbc = Mbc1::new(vec![0; 0x8000], ram, false);
        mbc.write_rom(0x0000, 0x0A); // enable
        mbc.write_rom(0x4000, 0x01); // ram_bank = 1
        mbc.write_rom(0x6000, 0x01); // mode 1
        assert_eq!(mbc.read_ram(0xA000), 0x22); // reads bank 1
    }

    #[test]
    fn mbc1_mode_0_locks_ram_to_bank_0() {
        let mut ram = vec![0u8; 0x8000];
        ram[0] = 0x11;
        ram[0x2000] = 0x22;
        let mut mbc = Mbc1::new(vec![0; 0x8000], ram, false);
        mbc.write_rom(0x0000, 0x0A); // enable
        mbc.write_rom(0x4000, 0x01); // ram_bank = 1
        mbc.write_rom(0x6000, 0x00); // mode 0
        assert_eq!(mbc.read_ram(0xA000), 0x11); // locked to bank 0
    }

    #[test]
    fn mbc1_deserialize_state_restores_registers() {
        let mut mbc = Mbc1::new(vec![0; 0x20000], vec![0; 0x2000], true);
        mbc.write_rom(0x0000, 0x0A); // enable
        mbc.write_rom(0x2000, 0x03); // rom_bank = 3
        mbc.write_rom(0x4000, 0x02); // ram_bank = 2
        mbc.write_rom(0x6000, 0x01); // mode 1

        let state = mbc.serialize_state();
        let mut restored = Mbc1::new(vec![0; 0x20000], vec![0; 0x2000], true);
        restored.deserialize_state(&state).expect("deserialize");

        let mut rom_set = vec![0u8; 0x20000];
        rom_set[3 * 0x4000] = 0xFF;
        let mut mbc2 = Mbc1::new(rom_set, vec![], false);
        mbc2.deserialize_state(&state).expect("deserialize 2");
        assert_eq!(mbc2.read_rom_n(0x4000), 0xFF);
    }

    #[test]
    fn serialize_state_round_trip() {
        let mut mbc = Mbc1::new(vec![0; 0x8000], vec![0x42; 0x2000], false);
        mbc.write_rom(0x2000, 0x05);
        let state = mbc.serialize_state();

        let mut restored = Mbc1::new(vec![0; 0x8000], vec![0; 0x2000], false);
        restored.deserialize_state(&state).expect("ok");
        assert_eq!(state, restored.serialize_state());
    }

    #[test]
    fn bank_mask_edge_cases() {
        assert_eq!(Mbc1::bank_mask(1), 0);
        assert_eq!(Mbc1::bank_mask(2), 1);
        assert_eq!(Mbc1::bank_mask(128), 127);
    }
}
