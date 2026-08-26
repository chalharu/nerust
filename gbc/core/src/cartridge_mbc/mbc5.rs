use super::{Mbc, MbcKind};

const MACHINE_STATE_SCHEMA_VERSION: u32 = 1;

/// MBC5 (up to 8 MiB ROM and 128 KiB RAM).
#[derive(Debug, Clone)]
pub struct Mbc5 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    ram_enabled: bool,
    rom_bank: u16,
    ram_bank: u8,
    battery: bool,
    rumble: bool,
    rumble_enabled: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Mbc5MachineState {
    schema_version: u32,
    ram_enabled: bool,
    rom_bank: u16,
    ram_bank: u8,
    rumble: bool,
    rumble_enabled: bool,
}

impl Mbc5 {
    pub fn new(rom: Vec<u8>, ram: Vec<u8>, battery: bool, rumble: bool) -> Self {
        Self {
            rom,
            ram,
            ram_enabled: false,
            rom_bank: 1,
            ram_bank: 0,
            battery,
            rumble,
            rumble_enabled: false,
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
                self.ram_bank = value & if self.rumble { 0x07 } else { 0x0F };
                self.rumble_enabled = self.rumble && value & 0x08 != 0;
            }
            _ => {}
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        if !self.ram_enabled || self.ram.is_empty() {
            return 0xFF;
        }
        let offset = (self.ram_bank as usize * 0x2000 + (addr as usize - 0xA000)) % self.ram.len();
        self.ram[offset]
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram_enabled || self.ram.is_empty() {
            return;
        }
        let offset = (self.ram_bank as usize * 0x2000 + (addr as usize - 0xA000)) % self.ram.len();
        self.ram[offset] = value;
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
        rmp_serde::to_vec_named(&Mbc5MachineState {
            schema_version: MACHINE_STATE_SCHEMA_VERSION,
            ram_enabled: self.ram_enabled,
            rom_bank: self.rom_bank,
            ram_bank: self.ram_bank,
            rumble: self.rumble,
            rumble_enabled: self.rumble_enabled,
        })
        .expect("MBC5 machine state should serialize")
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: Mbc5MachineState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != MACHINE_STATE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported MBC5 machine state version: {}",
                state.schema_version
            ));
        }
        if state.rumble != self.rumble {
            return Err("MBC5 machine rumble capability mismatch".into());
        }
        if state.rom_bank > 0x1FF {
            return Err("MBC5 machine ROM bank out of range".into());
        }
        let ram_bank_mask = if self.rumble { 0x07 } else { 0x0F };
        if state.ram_bank > ram_bank_mask {
            return Err("MBC5 machine RAM bank out of range".into());
        }
        if state.rumble_enabled && !self.rumble {
            return Err("MBC5 machine rumble state is unsupported".into());
        }

        self.ram_enabled = state.ram_enabled;
        self.rom_bank = state.rom_bank;
        self.ram_bank = state.ram_bank;
        self.rumble_enabled = state.rumble_enabled;
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
        let mbc = Mbc5::new(rom, vec![0; 0x2000], false, false);
        assert_eq!(mbc.read_rom0(0x0000), 0xAA);
        assert_eq!(mbc.read_rom_n(0x4000), 0xBB);
    }

    #[test]
    fn low_bank_register_switches_bank() {
        let mut rom = vec![0u8; 0x80000]; // 128 banks
        rom[5 * 0x4000] = 0xCC;
        let mut mbc = Mbc5::new(rom, vec![0; 0x2000], false, false);
        mbc.write_rom(0x2000, 0x05);
        assert_eq!(mbc.read_rom_n(0x4000), 0xCC);
    }

    #[test]
    fn high_bank_bit_toggles_above_256() {
        let mut rom = vec![0u8; 0x800000]; // 512 banks → 8 MiB
        rom[0x100 * 0x4000] = 0xDD;
        let mut mbc = Mbc5::new(rom, vec![0; 0x2000], false, false);
        mbc.write_rom(0x2000, 0x00);
        mbc.write_rom(0x3000, 0x01);
        assert_eq!(mbc.read_rom_n(0x4000), 0xDD);
    }

    #[test]
    fn ram_read_requires_enable() {
        let mut ram = vec![0u8; 0x2000];
        ram[0] = 0x77;
        let mut mbc = Mbc5::new(vec![0; 0x8000], ram, false, false);
        assert_eq!(mbc.read_ram(0xA000), 0xFF);
        mbc.write_rom(0x0000, 0x0A);
        assert_eq!(mbc.read_ram(0xA000), 0x77);
    }

    #[test]
    fn bank_zero_is_valid_in_switchable_window() {
        let mut rom = vec![0u8; 0x8000];
        rom[0] = 0xAA;
        rom[0x4000] = 0xBB;
        let mut mbc = Mbc5::new(rom, vec![], false, false);

        mbc.write_rom(0x2000, 0);

        assert_eq!(mbc.read_rom_n(0x4000), 0xAA);
    }

    #[test]
    fn selects_maximum_9_bit_rom_bank() {
        let mut rom = vec![0u8; 0x800000];
        rom[511 * 0x4000] = 0xEE;
        let mut mbc = Mbc5::new(rom, vec![], false, false);

        mbc.write_rom(0x2000, 0xFF);
        mbc.write_rom(0x3000, 0x01);

        assert_eq!(mbc.read_rom_n(0x4000), 0xEE);
    }

    #[test]
    fn rumble_bit_does_not_select_ram_bank_eight() {
        let mut ram = vec![0u8; 0x10000];
        ram[0x2000] = 0x11;
        let mut mbc = Mbc5::new(vec![0; 0x8000], ram, false, true);
        mbc.write_rom(0x0000, 0x0A);

        mbc.write_rom(0x4000, 0x09);

        assert_eq!(mbc.ram_bank, 1);
        assert!(mbc.rumble_enabled);
        assert_eq!(mbc.read_ram(0xA000), 0x11);
    }

    #[test]
    fn non_rumble_cartridge_selects_all_sixteen_ram_banks() {
        let mut ram = vec![0u8; 0x20000];
        ram[15 * 0x2000] = 0xF0;
        let mut mbc = Mbc5::new(vec![0; 0x8000], ram, false, false);
        mbc.write_rom(0x0000, 0x0A);

        mbc.write_rom(0x4000, 0x0F);

        assert_eq!(mbc.read_ram(0xA000), 0xF0);
    }

    #[test]
    fn unavailable_ram_banks_alias_physical_ram() {
        let mut mbc = Mbc5::new(vec![0; 0x8000], vec![0; 0x2000], false, false);
        mbc.write_rom(0x0000, 0x0A);
        mbc.write_rom(0x4000, 0x0F);

        mbc.write_ram(0xA000, 0x5A);

        mbc.write_rom(0x4000, 0);
        assert_eq!(mbc.read_ram(0xA000), 0x5A);
    }

    #[test]
    fn runtime_state_restores_rumble_and_banks() {
        let mut source = Mbc5::new(vec![0; 0x8000], vec![0; 0x10000], false, true);
        source.write_rom(0x0000, 0x0A);
        source.write_rom(0x2000, 0x34);
        source.write_rom(0x3000, 1);
        source.write_rom(0x4000, 0x0B);
        let state = source.serialize_state();
        let mut restored = Mbc5::new(vec![0; 0x8000], vec![0; 0x10000], false, true);

        restored.deserialize_state(&state).expect("restore");

        assert!(restored.ram_enabled);
        assert_eq!(restored.rom_bank, 0x134);
        assert_eq!(restored.ram_bank, 3);
        assert!(restored.rumble_enabled);
    }

    #[test]
    fn runtime_state_rejects_rumble_capability_mismatch_without_partial_restore() {
        let source = Mbc5::new(vec![0; 0x8000], vec![], false, true);
        let state = source.serialize_state();
        let mut target = Mbc5::new(vec![0; 0x8000], vec![], false, false);
        target.write_rom(0x2000, 7);

        assert!(target.deserialize_state(&state).is_err());
        assert_eq!(target.rom_bank, 7);
    }

    #[test]
    fn runtime_state_rejects_unknown_schema_version() {
        let source = Mbc5::new(vec![0; 0x8000], vec![], false, false);
        let mut state: Mbc5MachineState =
            rmp_serde::from_slice(&source.serialize_state()).expect("decode state");
        state.schema_version += 1;
        let bytes = rmp_serde::to_vec_named(&state).expect("encode state");
        let mut target = Mbc5::new(vec![0; 0x8000], vec![], false, false);

        assert!(target.deserialize_state(&bytes).is_err());
    }
}
