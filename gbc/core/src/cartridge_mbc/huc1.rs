use super::{Mbc, MbcKind};

const MACHINE_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct HuC1 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    rom_bank: u8,
    ram_bank: u8,
    ir_mode: bool,
    ir_output: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct HuC1MachineState {
    schema_version: u32,
    rom_bank: u8,
    ram_bank: u8,
    ir_mode: bool,
    ir_output: bool,
    #[serde(with = "serde_bytes")]
    ram: Vec<u8>,
}

impl HuC1 {
    pub fn new(rom: Vec<u8>, ram: Vec<u8>) -> Self {
        Self {
            rom,
            ram,
            rom_bank: 1,
            ram_bank: 0,
            ir_mode: false,
            ir_output: false,
        }
    }

    fn ram_offset(&self, addr: u16) -> Option<usize> {
        if self.ram.is_empty() {
            return None;
        }
        Some((usize::from(self.ram_bank) * 0x2000 + usize::from(addr - 0xA000)) % self.ram.len())
    }
}

impl Mbc for HuC1 {
    fn kind(&self) -> MbcKind {
        MbcKind::HuC1
    }

    fn read_rom0(&self, addr: u16) -> u8 {
        self.rom.get(usize::from(addr)).copied().unwrap_or(0xFF)
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        let offset = usize::from(self.rom_bank) * 0x4000 + usize::from(addr - 0x4000);
        self.rom.get(offset).copied().unwrap_or(0xFF)
    }

    fn write_rom(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => self.ir_mode = value == 0x0E,
            0x2000..=0x3FFF => self.rom_bank = value & 0x3F,
            0x4000..=0x5FFF => self.ram_bank = value & 0x03,
            _ => {}
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        if self.ir_mode {
            return 0xC0;
        }
        self.ram_offset(addr)
            .map_or(0xFF, |offset| self.ram[offset])
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if self.ir_mode {
            self.ir_output = value & 1 != 0;
        } else if let Some(offset) = self.ram_offset(addr) {
            self.ram[offset] = value;
        }
    }

    fn has_battery(&self) -> bool {
        true
    }

    fn ram_data(&self) -> Option<&[u8]> {
        (!self.ram.is_empty()).then_some(self.ram.as_slice())
    }

    fn ram_restore(&mut self, data: &[u8]) {
        if data.len() == self.ram.len() {
            self.ram.copy_from_slice(data);
        }
    }

    fn reset_runtime(&mut self) {
        self.rom_bank = 1;
        self.ram_bank = 0;
        self.ir_mode = false;
        self.ir_output = false;
    }

    fn serialize_state(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(&HuC1MachineState {
            schema_version: MACHINE_STATE_SCHEMA_VERSION,
            rom_bank: self.rom_bank,
            ram_bank: self.ram_bank,
            ir_mode: self.ir_mode,
            ir_output: self.ir_output,
            ram: self.ram.clone(),
        })
        .expect("HuC1 machine state should serialize")
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: HuC1MachineState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != MACHINE_STATE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported HuC1 machine state version: {}",
                state.schema_version
            ));
        }
        if state.rom_bank > 0x3F || state.ram_bank > 0x03 || state.ram.len() != self.ram.len() {
            return Err("invalid HuC1 machine state".into());
        }

        self.rom_bank = state.rom_bank;
        self.ram_bank = state.ram_bank;
        self.ir_mode = state.ir_mode;
        self.ir_output = state.ir_output;
        self.ram = state.ram;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    fn huc1() -> HuC1 {
        let mut rom = vec![0; 64 * 0x4000];
        for bank in 0..64 {
            rom[bank * 0x4000] = bank as u8;
        }
        HuC1::new(rom, vec![0; 4 * 0x2000])
    }

    #[test]
    fn switches_rom_and_ram_banks_without_mbc1_remap() {
        let mut mbc = huc1();
        mbc.write_rom(0x2000, 0);
        assert_eq!(mbc.read_rom_n(0x4000), 0);
        mbc.write_rom(0x2000, 0xFF);
        assert_eq!(mbc.read_rom_n(0x4000), 63);

        mbc.write_rom(0x4000, 3);
        mbc.write_ram(0xA000, 0xA3);
        mbc.write_rom(0x4000, 0);
        assert_eq!(mbc.read_ram(0xA000), 0);
        mbc.write_rom(0x4000, 3);
        assert_eq!(mbc.read_ram(0xA000), 0xA3);
    }

    #[test]
    fn only_exact_0x0e_selects_deterministic_ir() {
        let mut mbc = huc1();
        mbc.write_ram(0xA000, 0x42);
        for value in [0x00, 0x0A, 0x1E] {
            mbc.write_rom(0, value);
            assert_eq!(mbc.read_ram(0xA000), 0x42);
        }

        mbc.write_rom(0, 0x0E);
        assert_eq!(mbc.read_ram(0xA000), 0xC0);
        assert_eq!(mbc.read_ram(0xBFFF), 0xC0);
        mbc.write_ram(0xB123, 1);
        assert!(mbc.ir_output);
        mbc.write_ram(0xA000, 2);
        assert!(!mbc.ir_output);
    }

    #[test]
    fn reset_preserves_ram_but_clears_runtime_state() {
        let mut mbc = huc1();
        mbc.write_ram(0xA000, 0x55);
        mbc.write_rom(0x2000, 12);
        mbc.write_rom(0x4000, 2);
        mbc.write_rom(0, 0x0E);
        mbc.write_ram(0xA000, 1);

        mbc.reset_runtime();

        assert_eq!(mbc.rom_bank, 1);
        assert_eq!(mbc.ram_bank, 0);
        assert!(!mbc.ir_mode);
        assert!(!mbc.ir_output);
        assert_eq!(mbc.read_ram(0xA000), 0x55);
    }

    #[test]
    fn machine_and_persistent_state_round_trip() {
        let mut source = huc1();
        source.write_rom(0x2000, 7);
        source.write_rom(0x4000, 2);
        source.write_ram(0xA000, 0x77);
        source.write_rom(0, 0x0E);
        source.write_ram(0xA000, 1);

        let machine = source.serialize_state();
        let persistent = source
            .export_persistent_state(SystemTime::UNIX_EPOCH)
            .unwrap()
            .unwrap();

        let mut machine_target = huc1();
        machine_target.deserialize_state(&machine).unwrap();
        assert_eq!(machine_target.rom_bank, 7);
        assert_eq!(machine_target.ram_bank, 2);
        assert!(machine_target.ir_mode);
        assert!(machine_target.ir_output);

        let mut persistent_target = huc1();
        persistent_target
            .import_persistent_state(&persistent)
            .unwrap();
        persistent_target.write_rom(0x4000, 2);
        assert_eq!(persistent_target.read_ram(0xA000), 0x77);
        assert!(!persistent_target.ir_mode);
    }

    #[test]
    fn invalid_machine_state_is_transactional() {
        let source = huc1();
        let mut state: HuC1MachineState = rmp_serde::from_slice(&source.serialize_state()).unwrap();
        state.rom_bank = 0x40;
        let data = rmp_serde::to_vec_named(&state).unwrap();
        let mut target = huc1();
        target.rom_bank = 9;

        assert!(target.deserialize_state(&data).is_err());
        assert_eq!(target.rom_bank, 9);
    }
}
