use super::{Mbc, MbcKind};

const ROM_BANK_SIZE: usize = 0x4000;
const RAM_BANK_SIZE: usize = 0x2000;

#[derive(Debug, Clone)]
pub struct Mmm01 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    battery: bool,
    mapped: bool,
    ram_enabled: bool,
    rom_bank_low: u8,
    rom_bank_mid: u8,
    rom_bank_high: u8,
    rom_bank_mask: u8,
    ram_bank_low: u8,
    ram_bank_high: u8,
    ram_bank_mask: u8,
    mbc1_mode: bool,
    mbc1_mode_write_disabled: bool,
    multiplex: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Mmm01MachineState {
    schema_version: u32,
    mapped: bool,
    ram_enabled: bool,
    rom_bank_low: u8,
    rom_bank_mid: u8,
    rom_bank_high: u8,
    rom_bank_mask: u8,
    ram_bank_low: u8,
    ram_bank_high: u8,
    ram_bank_mask: u8,
    mbc1_mode: bool,
    mbc1_mode_write_disabled: bool,
    multiplex: bool,
    #[serde(with = "serde_bytes")]
    ram: Vec<u8>,
}

impl Mmm01 {
    pub fn new(rom: Vec<u8>, ram: Vec<u8>, battery: bool) -> Self {
        Self {
            rom,
            ram,
            battery,
            mapped: false,
            ram_enabled: false,
            rom_bank_low: 0,
            rom_bank_mid: 0,
            rom_bank_high: 0,
            rom_bank_mask: 0,
            ram_bank_low: 0,
            ram_bank_high: 0,
            ram_bank_mask: 0,
            mbc1_mode: false,
            mbc1_mode_write_disabled: false,
            multiplex: false,
        }
    }

    fn menu_banks(&self) -> (usize, usize) {
        let bank_count = self.rom.len() / ROM_BANK_SIZE;
        (bank_count.saturating_sub(2), bank_count.saturating_sub(1))
    }

    fn mapped_rom_banks(&self) -> (usize, usize) {
        let high = usize::from(self.rom_bank_high) << 7;
        let fixed_low = usize::from(self.rom_bank_low & self.rom_bank_mask);
        let selected_low = if self.rom_bank_low & !self.rom_bank_mask & 0x1F == 0 {
            self.rom_bank_low | 1
        } else {
            self.rom_bank_low
        };

        if self.multiplex {
            let mid0 = if self.mbc1_mode {
                self.ram_bank_low
            } else {
                self.ram_bank_low & self.ram_bank_mask
            };
            (
                high | (usize::from(mid0) << 5) | fixed_low,
                high | (usize::from(self.ram_bank_low) << 5) | usize::from(selected_low),
            )
        } else {
            let mid = usize::from(self.rom_bank_mid) << 5;
            (
                high | mid | fixed_low,
                high | mid | usize::from(selected_low),
            )
        }
    }

    fn rom_banks(&self) -> (usize, usize) {
        if self.mapped {
            self.mapped_rom_banks()
        } else {
            self.menu_banks()
        }
    }

    fn ram_bank(&self) -> usize {
        let low = if self.multiplex {
            self.rom_bank_mid
        } else if self.mbc1_mode {
            self.ram_bank_low
        } else {
            self.ram_bank_low & self.ram_bank_mask
        };
        (usize::from(self.ram_bank_high) << 2) | usize::from(low)
    }

    fn read_rom_bank(&self, bank: usize, offset: usize) -> u8 {
        bank.checked_mul(ROM_BANK_SIZE)
            .and_then(|start| start.checked_add(offset))
            .and_then(|index| self.rom.get(index))
            .copied()
            .unwrap_or(0xFF)
    }

    fn ram_offset(&self, addr: u16) -> Option<usize> {
        if !self.mapped || !self.ram_enabled || self.ram.is_empty() {
            return None;
        }
        let offset = self
            .ram_bank()
            .checked_mul(RAM_BANK_SIZE)?
            .checked_add(usize::from(addr - 0xA000))?;
        Some(offset % self.ram.len())
    }

    fn update_masked(current: u8, value: u8, mask: u8, field_mask: u8) -> u8 {
        ((current & mask) | (value & !mask)) & field_mask
    }
}

impl Mbc for Mmm01 {
    fn kind(&self) -> MbcKind {
        MbcKind::Mmm01
    }

    fn read_rom0(&self, addr: u16) -> u8 {
        let (bank, _) = self.rom_banks();
        self.read_rom_bank(bank, usize::from(addr))
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        let (_, bank) = self.rom_banks();
        self.read_rom_bank(bank, usize::from(addr - 0x4000))
    }

    fn write_rom(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => {
                self.ram_enabled = value & 0x0F == 0x0A;
                if !self.mapped {
                    self.ram_bank_mask = (value >> 4) & 0x03;
                    self.mapped = value & 0x40 != 0;
                }
            }
            0x2000..=0x3FFF => {
                self.rom_bank_low =
                    Self::update_masked(self.rom_bank_low, value, self.rom_bank_mask, 0x1F);
                if !self.mapped {
                    self.rom_bank_mid = (value >> 5) & 0x03;
                }
            }
            0x4000..=0x5FFF => {
                self.ram_bank_low =
                    Self::update_masked(self.ram_bank_low, value, self.ram_bank_mask, 0x03);
                if !self.mapped {
                    self.ram_bank_high = (value >> 2) & 0x03;
                    self.rom_bank_high = (value >> 4) & 0x03;
                    self.mbc1_mode_write_disabled = value & 0x40 != 0;
                }
            }
            0x6000..=0x7FFF => {
                if !self.mbc1_mode_write_disabled {
                    self.mbc1_mode = value & 1 != 0;
                }
                if !self.mapped {
                    self.rom_bank_mask = (value >> 1) & 0x1E;
                    self.multiplex = value & 0x40 != 0;
                }
            }
            _ => {}
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        self.ram_offset(addr)
            .and_then(|offset| self.ram.get(offset))
            .copied()
            .unwrap_or(0xFF)
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if let Some(offset) = self.ram_offset(addr) {
            self.ram[offset] = value;
        }
    }

    fn has_battery(&self) -> bool {
        self.battery
    }

    fn ram_data(&self) -> Option<&[u8]> {
        (!self.ram.is_empty()).then_some(&self.ram)
    }

    fn ram_restore(&mut self, data: &[u8]) {
        if data.len() == self.ram.len() {
            self.ram.copy_from_slice(data);
        }
    }

    fn reset_runtime(&mut self) {
        let ram = std::mem::take(&mut self.ram);
        let rom = std::mem::take(&mut self.rom);
        let battery = self.battery;
        *self = Self::new(rom, ram, battery);
    }

    fn serialize_state(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(&Mmm01MachineState {
            schema_version: 1,
            mapped: self.mapped,
            ram_enabled: self.ram_enabled,
            rom_bank_low: self.rom_bank_low,
            rom_bank_mid: self.rom_bank_mid,
            rom_bank_high: self.rom_bank_high,
            rom_bank_mask: self.rom_bank_mask,
            ram_bank_low: self.ram_bank_low,
            ram_bank_high: self.ram_bank_high,
            ram_bank_mask: self.ram_bank_mask,
            mbc1_mode: self.mbc1_mode,
            mbc1_mode_write_disabled: self.mbc1_mode_write_disabled,
            multiplex: self.multiplex,
            ram: self.ram.clone(),
        })
        .expect("MMM01 machine state should serialize")
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: Mmm01MachineState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != 1 {
            return Err(format!(
                "unsupported MMM01 machine state version: {}",
                state.schema_version
            ));
        }
        if state.ram.len() != self.ram.len()
            || state.rom_bank_low > 0x1F
            || state.rom_bank_mid > 0x03
            || state.rom_bank_high > 0x03
            || state.rom_bank_mask & !0x1E != 0
            || state.ram_bank_low > 0x03
            || state.ram_bank_high > 0x03
            || state.ram_bank_mask > 0x03
        {
            return Err("invalid MMM01 machine state".into());
        }

        self.mapped = state.mapped;
        self.ram_enabled = state.ram_enabled;
        self.rom_bank_low = state.rom_bank_low;
        self.rom_bank_mid = state.rom_bank_mid;
        self.rom_bank_high = state.rom_bank_high;
        self.rom_bank_mask = state.rom_bank_mask;
        self.ram_bank_low = state.ram_bank_low;
        self.ram_bank_high = state.ram_bank_high;
        self.ram_bank_mask = state.ram_bank_mask;
        self.mbc1_mode = state.mbc1_mode;
        self.mbc1_mode_write_disabled = state.mbc1_mode_write_disabled;
        self.multiplex = state.multiplex;
        self.ram = state.ram;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    fn banked_rom(bank_count: usize) -> Vec<u8> {
        let mut rom = vec![0; bank_count * ROM_BANK_SIZE];
        for (bank, chunk) in rom.chunks_exact_mut(ROM_BANK_SIZE).enumerate() {
            chunk.fill(bank as u8);
        }
        rom
    }

    #[test]
    fn starts_with_last_two_menu_banks() {
        let mapper = Mmm01::new(banked_rom(16), Vec::new(), false);
        assert_eq!(mapper.read_rom0(0), 14);
        assert_eq!(mapper.read_rom_n(0x4000), 15);
    }

    #[test]
    fn mapping_enable_is_one_way_and_locks_extended_registers() {
        let mut mapper = Mmm01::new(banked_rom(256), Vec::new(), false);
        mapper.write_rom(0x2000, 0x22);
        mapper.write_rom(0x0000, 0x40);
        assert_eq!(mapper.read_rom0(0), 0x20);
        assert_eq!(mapper.read_rom_n(0x4000), 0x22);

        mapper.write_rom(0x2000, 0x42);
        mapper.write_rom(0x0000, 0);
        assert_eq!(mapper.read_rom0(0), 0x20);
        assert_eq!(mapper.read_rom_n(0x4000), 0x22);
    }

    #[test]
    fn rom_mask_preserves_game_select_bits_and_remaps_game_bank_zero() {
        let mut mapper = Mmm01::new(banked_rom(64), Vec::new(), false);
        mapper.write_rom(0x2000, 0x10);
        mapper.write_rom(0x6000, 0x20);
        mapper.write_rom(0x0000, 0x40);
        mapper.write_rom(0x2000, 0x00);
        assert_eq!(mapper.read_rom0(0), 0x10);
        assert_eq!(mapper.read_rom_n(0x4000), 0x11);
    }

    #[test]
    fn multiplex_mode_uses_ram_low_bits_for_rom_banking() {
        let mut mapper = Mmm01::new(banked_rom(128), Vec::new(), false);
        mapper.write_rom(0x6000, 0x40);
        mapper.write_rom(0x4000, 0x02);
        mapper.write_rom(0x2000, 0x03);
        mapper.write_rom(0x0000, 0x40);
        assert_eq!(mapper.read_rom0(0), 0);
        assert_eq!(mapper.read_rom_n(0x4000), 0x43);
        mapper.write_rom(0x6000, 1);
        assert_eq!(mapper.read_rom0(0), 0x40);
    }

    #[test]
    fn ram_banking_and_persistence_are_independent_from_runtime_state() {
        let mut mapper = Mmm01::new(banked_rom(8), vec![0; 0x8000], true);
        mapper.write_rom(0x0000, 0x4A);
        mapper.write_rom(0x6000, 1);
        mapper.write_rom(0x4000, 2);
        mapper.write_ram(0xA000, 0x5A);
        assert_eq!(mapper.read_ram(0xA000), 0x5A);

        let persistent = mapper
            .export_persistent_state(SystemTime::UNIX_EPOCH)
            .unwrap()
            .unwrap();
        let machine = mapper.serialize_state();
        let mut restored = Mmm01::new(banked_rom(8), vec![0; 0x8000], true);
        restored.deserialize_state(&machine).unwrap();
        assert_eq!(restored.read_ram(0xA000), 0x5A);

        restored.reset_runtime();
        restored.import_persistent_state(&persistent).unwrap();
        assert_eq!(restored.read_ram(0xA000), 0xFF);
        restored.write_rom(0x0000, 0x4A);
        restored.write_rom(0x6000, 1);
        restored.write_rom(0x4000, 2);
        assert_eq!(restored.read_ram(0xA000), 0x5A);
    }

    #[test]
    fn invalid_machine_state_is_transactional() {
        let mut mapper = Mmm01::new(banked_rom(8), vec![0; 0x2000], false);
        mapper.write_rom(0x0000, 0x40);
        let before = mapper.serialize_state();
        let mut state: Mmm01MachineState = rmp_serde::from_slice(&before).unwrap();
        state.rom_bank_low = 0xFF;
        let invalid = rmp_serde::to_vec_named(&state).unwrap();
        assert!(mapper.deserialize_state(&invalid).is_err());
        assert_eq!(mapper.serialize_state(), before);
    }
}
