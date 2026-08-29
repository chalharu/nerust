use std::time::SystemTime;

use super::{Mbc, MbcKind, huc3_rtc::HuC3Rtc};

const MACHINE_STATE_SCHEMA_VERSION: u32 = 1;
const PERSISTENT_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct HuC3 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    rom_bank: u8,
    ram_bank: u8,
    mode: u8,
    mailbox: HuC3Mailbox,
    rtc: HuC3Rtc,
    ir_output: bool,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct HuC3Mailbox {
    command: u8,
    argument: u8,
    response: u8,
    address: u8,
}

impl HuC3Mailbox {
    fn valid(self) -> bool {
        self.command <= 0x07 && self.argument <= 0x0F && self.response <= 0x0F
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct HuC3MachineState {
    schema_version: u32,
    rom_bank: u8,
    ram_bank: u8,
    mode: u8,
    mailbox: HuC3Mailbox,
    #[serde(with = "serde_bytes")]
    rtc_memory: Vec<u8>,
    subminute_clocks: u32,
    ir_output: bool,
    #[serde(with = "serde_bytes")]
    ram: Vec<u8>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct HuC3PersistentState {
    schema_version: u32,
    kind: MbcKind,
    #[serde(with = "serde_bytes")]
    ram: Vec<u8>,
    #[serde(with = "serde_bytes")]
    rtc_memory: Vec<u8>,
    subminute_clocks: u32,
    saved_at_unix_seconds: u64,
}

impl HuC3 {
    pub fn new(rom: Vec<u8>, ram: Vec<u8>) -> Self {
        Self {
            rom,
            ram,
            rom_bank: 1,
            ram_bank: 0,
            mode: 0,
            mailbox: HuC3Mailbox::default(),
            rtc: HuC3Rtc::new(),
            ir_output: false,
        }
    }

    fn ram_offset(&self, addr: u16) -> Option<usize> {
        if self.ram.is_empty() {
            return None;
        }
        Some((usize::from(self.ram_bank) * 0x2000 + usize::from(addr - 0xA000)) % self.ram.len())
    }

    fn execute_command(&mut self) {
        self.mailbox.response = match self.mailbox.command {
            0x01 => {
                let response = self.rtc.read(self.mailbox.address);
                self.mailbox.address = self.mailbox.address.wrapping_add(1);
                response
            }
            0x03 => {
                self.rtc.write(self.mailbox.address, self.mailbox.argument);
                self.mailbox.address = self.mailbox.address.wrapping_add(1);
                0
            }
            0x04 => {
                self.mailbox.address = (self.mailbox.address & 0xF0) | self.mailbox.argument;
                0
            }
            0x05 => {
                self.mailbox.address = (self.mailbox.address & 0x0F) | (self.mailbox.argument << 4);
                0
            }
            0x06 => self.execute_extended_command(),
            _ => 0,
        };
    }

    fn execute_extended_command(&mut self) -> u8 {
        match self.mailbox.argument {
            0x00 => self.rtc.copy_current_to_transfer(),
            0x01 => {
                self.rtc.copy_transfer_to_current();
            }
            0x02 => return 1,
            0x0E => {}
            _ => {}
        }
        0
    }
}

impl Mbc for HuC3 {
    fn kind(&self) -> MbcKind {
        MbcKind::HuC3
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
            0x0000..=0x1FFF => self.mode = value & 0x0F,
            0x2000..=0x3FFF => self.rom_bank = value & 0x7F,
            0x4000..=0x5FFF => self.ram_bank = value & 0x03,
            _ => {}
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        match self.mode {
            0x00 | 0x0A => self
                .ram_offset(addr)
                .map_or(0xFF, |offset| self.ram[offset]),
            0x0C => 0x80 | (self.mailbox.command << 4) | self.mailbox.response,
            0x0D => 0x81,
            0x0E => 0x80,
            _ => 0xFF,
        }
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        match self.mode {
            0x0A => {
                if let Some(offset) = self.ram_offset(addr) {
                    self.ram[offset] = value;
                }
            }
            0x0B => {
                self.mailbox.command = (value >> 4) & 0x07;
                self.mailbox.argument = value & 0x0F;
            }
            0x0D if value & 1 == 0 => self.execute_command(),
            0x0E => self.ir_output = value & 1 != 0,
            _ => {}
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

    fn has_rtc(&self) -> bool {
        true
    }

    fn step_clock(&mut self) {
        self.rtc.step_clock();
    }

    fn sync_rtc(&mut self, now: SystemTime) {
        self.rtc.sync(now);
    }

    fn sync_rtc_from(&mut self, saved_at: SystemTime, now: SystemTime) {
        self.rtc.sync_from(saved_at, now);
    }

    fn reset_runtime(&mut self) {
        self.rom_bank = 1;
        self.ram_bank = 0;
        self.mode = 0;
        self.mailbox = HuC3Mailbox::default();
        self.ir_output = false;
    }

    fn export_persistent_state(&self, now: SystemTime) -> Result<Option<Vec<u8>>, String> {
        let saved_at_unix_seconds = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        rmp_serde::to_vec_named(&HuC3PersistentState {
            schema_version: PERSISTENT_STATE_SCHEMA_VERSION,
            kind: self.kind(),
            ram: self.ram.clone(),
            rtc_memory: self.rtc.memory().to_vec(),
            subminute_clocks: self.rtc.subminute_clocks(),
            saved_at_unix_seconds,
        })
        .map(Some)
        .map_err(|error| error.to_string())
    }

    fn import_persistent_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: HuC3PersistentState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != PERSISTENT_STATE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported HuC3 persistent state version: {}",
                state.schema_version
            ));
        }
        if state.kind != self.kind() {
            return Err("HuC3 persistent state kind mismatch".into());
        }
        if state.ram.len() != self.ram.len() {
            return Err(format!(
                "HuC3 persistent RAM length mismatch: expected {}, got {}",
                self.ram.len(),
                state.ram.len()
            ));
        }
        let rtc = HuC3Rtc::from_state(
            state.rtc_memory,
            state.subminute_clocks,
            Some(state.saved_at_unix_seconds),
        )?;

        self.ram = state.ram;
        self.rtc = rtc;
        Ok(())
    }

    fn serialize_state(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(&HuC3MachineState {
            schema_version: MACHINE_STATE_SCHEMA_VERSION,
            rom_bank: self.rom_bank,
            ram_bank: self.ram_bank,
            mode: self.mode,
            mailbox: self.mailbox,
            rtc_memory: self.rtc.memory().to_vec(),
            subminute_clocks: self.rtc.subminute_clocks(),
            ir_output: self.ir_output,
            ram: self.ram.clone(),
        })
        .expect("HuC3 machine state should serialize")
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: HuC3MachineState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != MACHINE_STATE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported HuC3 machine state version: {}",
                state.schema_version
            ));
        }
        if state.rom_bank > 0x7F
            || state.ram_bank > 0x03
            || state.mode > 0x0F
            || !state.mailbox.valid()
            || state.ram.len() != self.ram.len()
        {
            return Err("invalid HuC3 machine state".into());
        }
        let rtc = HuC3Rtc::from_state(state.rtc_memory, state.subminute_clocks, None)?;

        self.rom_bank = state.rom_bank;
        self.ram_bank = state.ram_bank;
        self.mode = state.mode;
        self.mailbox = state.mailbox;
        self.rtc = rtc;
        self.ir_output = state.ir_output;
        self.ram = state.ram;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn huc3() -> HuC3 {
        let mut rom = vec![0; 128 * 0x4000];
        for bank in 0..128 {
            rom[bank * 0x4000] = bank as u8;
        }
        HuC3::new(rom, vec![0; 4 * 0x2000])
    }

    fn command(mbc: &mut HuC3, command: u8, argument: u8) -> u8 {
        mbc.write_rom(0, 0x0B);
        mbc.write_ram(0xA000, command << 4 | argument);
        mbc.write_rom(0, 0x0D);
        mbc.write_ram(0xBFFF, 0xFE);
        mbc.write_rom(0, 0x0C);
        mbc.read_ram(0xA123)
    }

    fn set_address(mbc: &mut HuC3, address: u8) {
        command(mbc, 4, address & 0x0F);
        command(mbc, 5, address >> 4);
    }

    #[test]
    fn switches_banks_and_distinguishes_read_only_ram() {
        let mut mbc = huc3();
        mbc.write_rom(0x2000, 0);
        assert_eq!(mbc.read_rom_n(0x4000), 0);
        mbc.write_rom(0x2000, 0xFF);
        assert_eq!(mbc.read_rom_n(0x4000), 127);

        mbc.write_rom(0x4000, 3);
        mbc.write_rom(0, 0x0A);
        mbc.write_ram(0xA000, 0xA3);
        mbc.write_rom(0, 0);
        mbc.write_ram(0xA000, 0x55);
        assert_eq!(mbc.read_ram(0xA000), 0xA3);
    }

    #[test]
    fn mailbox_executes_only_when_semaphore_is_cleared() {
        let mut mbc = huc3();
        set_address(&mut mbc, 0xFF);
        mbc.write_rom(0, 0x0B);
        mbc.write_ram(0xA000, 0x3A);
        assert_eq!(mbc.rtc.read(0xFF), 0);
        mbc.write_rom(0, 0x0D);
        mbc.write_ram(0xA000, 1);
        assert_eq!(mbc.rtc.read(0xFF), 0);
        mbc.write_ram(0xA000, 0xFE);
        assert_eq!(mbc.rtc.read(0xFF), 0x0A);
        assert_eq!(mbc.mailbox.address, 0);
        assert_eq!(mbc.read_ram(0xA000), 0x81);

        set_address(&mut mbc, 0xFF);
        assert_eq!(command(&mut mbc, 1, 0) & 0x0F, 0x0A);
    }

    #[test]
    fn extended_status_and_deterministic_ir_work() {
        let mut mbc = huc3();
        assert_eq!(command(&mut mbc, 6, 2) & 0x0F, 1);

        mbc.write_rom(0, 0x0E);
        assert_eq!(mbc.read_ram(0xA000), 0x80);
        mbc.write_ram(0xBFFF, 1);
        assert!(mbc.ir_output);
        mbc.write_ram(0xA000, 2);
        assert!(!mbc.ir_output);
    }

    #[test]
    fn current_time_commands_transfer_atomically() {
        let mut mbc = huc3();
        set_address(&mut mbc, 0);
        for nibble in [5, 0, 0, 2, 0, 0, 0] {
            command(&mut mbc, 3, nibble);
        }
        command(&mut mbc, 6, 1);
        assert_eq!(mbc.rtc.read(0x10), 5);
        assert_eq!(mbc.rtc.read(0x13), 2);

        command(&mut mbc, 6, 0);
        assert_eq!(mbc.rtc.read(0), 5);
        assert_eq!(mbc.rtc.read(3), 2);
    }

    #[test]
    fn machine_and_persistent_state_round_trip() {
        let mut source = huc3();
        source.write_rom(0, 0x0A);
        source.write_rom(0x4000, 2);
        source.write_ram(0xA000, 0x66);
        set_address(&mut source, 0x42);
        command(&mut source, 3, 0x0C);
        source.write_rom(0, 0x0E);
        source.write_ram(0xA000, 1);

        let machine = source.serialize_state();
        let persistent = source
            .export_persistent_state(UNIX_EPOCH + Duration::from_secs(100))
            .unwrap()
            .unwrap();

        let mut machine_target = huc3();
        machine_target.deserialize_state(&machine).unwrap();
        assert_eq!(machine_target.rtc.read(0x42), 0x0C);
        assert!(machine_target.ir_output);

        let mut persistent_target = huc3();
        persistent_target
            .import_persistent_state(&persistent)
            .unwrap();
        persistent_target.write_rom(0, 0x0A);
        persistent_target.write_rom(0x4000, 2);
        assert_eq!(persistent_target.read_ram(0xA000), 0x66);
        assert_eq!(persistent_target.rtc.read(0x42), 0x0C);
        assert!(!persistent_target.ir_output);
    }

    #[test]
    fn invalid_persistent_state_is_transactional() {
        let source = huc3();
        let data = source.export_persistent_state(UNIX_EPOCH).unwrap().unwrap();
        let mut state: HuC3PersistentState = rmp_serde::from_slice(&data).unwrap();
        state.rtc_memory[0] = 0x10;
        let data = rmp_serde::to_vec_named(&state).unwrap();
        let mut target = huc3();
        target.ram[0] = 0x55;

        assert!(target.import_persistent_state(&data).is_err());
        assert_eq!(target.ram[0], 0x55);
    }
}
