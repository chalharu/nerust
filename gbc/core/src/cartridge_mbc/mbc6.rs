use std::time::SystemTime;

use super::{Mbc, MbcKind};

const MACHINE_STATE_SCHEMA_VERSION: u32 = 1;
const PERSISTENT_STATE_SCHEMA_VERSION: u32 = 1;
const SRAM_SIZE: usize = 0x8000;
const FLASH_SIZE: usize = 0x10_0000;
const HIDDEN_SIZE: usize = 0x100;
const ROM_BANK_SIZE: usize = 0x2000;
const RAM_BANK_SIZE: usize = 0x1000;
const FLASH_SECTOR_SIZE: usize = 0x20_000;
const PROGRAM_BLOCK_SIZE: usize = 0x80;
const UNLOCK_ADDRESS_1: usize = 0x5555;
const UNLOCK_ADDRESS_2: usize = 0x2AAA;

#[derive(Debug, Clone)]
pub struct Mbc6 {
    rom: Vec<u8>,
    sram: Vec<u8>,
    flash: Vec<u8>,
    hidden: Vec<u8>,
    ram_enabled: bool,
    ram_bank_a: u8,
    ram_bank_b: u8,
    flash_enabled: bool,
    flash_write_enabled: bool,
    bank_a: u8,
    bank_b: u8,
    flash_a: bool,
    flash_b: bool,
    sector0_protected: bool,
    flash_mode: FlashMode,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum FlashMode {
    ReadArray,
    UnlockSecond,
    Command,
    FollowupFirst { command: u8 },
    FollowupSecond { command: u8 },
    FollowupCommand { command: u8 },
    Id,
    HiddenRead,
    Program(ProgramBuffer),
    HiddenProgram(ProgramBuffer),
    Status { failed: bool },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ProgramBuffer {
    block_start: Option<usize>,
    data: Vec<u8>,
    written: Vec<bool>,
    last_address: Option<usize>,
}

impl ProgramBuffer {
    fn new() -> Self {
        Self {
            block_start: None,
            data: vec![0xFF; PROGRAM_BLOCK_SIZE],
            written: vec![false; PROGRAM_BLOCK_SIZE],
            last_address: None,
        }
    }

    fn write(&mut self, address: usize, value: u8) -> ProgramAction {
        let block_start = *self
            .block_start
            .get_or_insert(address & !(PROGRAM_BLOCK_SIZE - 1));
        if !(block_start..block_start + PROGRAM_BLOCK_SIZE).contains(&address) {
            return ProgramAction::Failed;
        }
        let offset = address - block_start;
        if self.written[offset] {
            return if self.written.iter().all(|written| *written)
                && self.last_address == Some(address)
            {
                ProgramAction::Commit
            } else {
                ProgramAction::Failed
            };
        }
        self.data[offset] = value;
        self.written[offset] = true;
        self.last_address = Some(address);
        ProgramAction::Collecting
    }

    fn valid(&self, storage_len: usize) -> bool {
        self.data.len() == PROGRAM_BLOCK_SIZE
            && self.written.len() == PROGRAM_BLOCK_SIZE
            && self.block_start.is_none_or(|start| {
                start % PROGRAM_BLOCK_SIZE == 0
                    && start
                        .checked_add(PROGRAM_BLOCK_SIZE)
                        .is_some_and(|end| end <= storage_len)
            })
            && self
                .last_address
                .is_none_or(|address| address < storage_len)
    }
}

enum ProgramAction {
    Collecting,
    Commit,
    Failed,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Mbc6MachineState {
    schema_version: u32,
    sram: Vec<u8>,
    flash: Vec<u8>,
    hidden: Vec<u8>,
    ram_enabled: bool,
    ram_bank_a: u8,
    ram_bank_b: u8,
    flash_enabled: bool,
    flash_write_enabled: bool,
    bank_a: u8,
    bank_b: u8,
    flash_a: bool,
    flash_b: bool,
    sector0_protected: bool,
    flash_mode: FlashMode,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Mbc6PersistentState {
    schema_version: u32,
    kind: MbcKind,
    #[serde(with = "serde_bytes")]
    sram: Vec<u8>,
    #[serde(with = "serde_bytes")]
    flash: Vec<u8>,
    #[serde(with = "serde_bytes")]
    hidden: Vec<u8>,
    sector0_protected: bool,
}

impl Mbc6 {
    pub fn new(rom: Vec<u8>) -> Self {
        Self {
            rom,
            sram: vec![0; SRAM_SIZE],
            flash: vec![0xFF; FLASH_SIZE],
            hidden: vec![0xFF; HIDDEN_SIZE],
            ram_enabled: false,
            ram_bank_a: 0,
            ram_bank_b: 1,
            flash_enabled: false,
            flash_write_enabled: false,
            bank_a: 2,
            bank_b: 3,
            flash_a: false,
            flash_b: false,
            sector0_protected: false,
            flash_mode: FlashMode::ReadArray,
        }
    }

    fn selected_window(&self, addr: u16) -> (u8, bool) {
        if addr < 0x6000 {
            (self.bank_a, self.flash_a)
        } else {
            (self.bank_b, self.flash_b)
        }
    }

    fn banked_offset(bank: u8, addr: u16) -> usize {
        usize::from(bank) * ROM_BANK_SIZE + (usize::from(addr) & (ROM_BANK_SIZE - 1))
    }

    fn flash_read(&self, address: usize) -> u8 {
        match self.flash_mode {
            FlashMode::Id => match address & (ROM_BANK_SIZE - 1) {
                0 => 0xC2,
                1 => 0x81,
                _ => 0xFF,
            },
            FlashMode::HiddenRead => self.hidden[address & (HIDDEN_SIZE - 1)],
            FlashMode::Status { failed } => {
                0x80 | (u8::from(failed) << 4) | (u8::from(self.sector0_protected) << 1)
            }
            _ => self.flash.get(address).copied().unwrap_or(0xFF),
        }
    }

    fn start_sequence_or_reset(&mut self, address: usize, value: u8) {
        self.flash_mode = if address == UNLOCK_ADDRESS_1 && value == 0xAA {
            FlashMode::UnlockSecond
        } else {
            FlashMode::ReadArray
        };
    }

    fn write_flash(&mut self, address: usize, value: u8) {
        if value == 0xF0 {
            self.flash_mode = FlashMode::ReadArray;
            return;
        }

        let mode = std::mem::replace(&mut self.flash_mode, FlashMode::ReadArray);
        match mode {
            FlashMode::ReadArray => self.start_sequence_or_reset(address, value),
            mode @ (FlashMode::Id | FlashMode::HiddenRead | FlashMode::Status { .. }) => {
                self.flash_mode = mode;
            }
            FlashMode::UnlockSecond => self.write_unlock_second(address, value),
            FlashMode::Command => self.write_command(address, value),
            FlashMode::FollowupFirst { command } => {
                self.write_followup_first(command, address, value)
            }
            FlashMode::FollowupSecond { command } => {
                self.write_followup_second(command, address, value)
            }
            FlashMode::FollowupCommand { command } => self.finish_followup(command, address, value),
            FlashMode::Program(buffer) => self.write_program(buffer, address, value, false),
            FlashMode::HiddenProgram(buffer) => self.write_program(buffer, address, value, true),
        }
    }

    fn write_unlock_second(&mut self, address: usize, value: u8) {
        self.flash_mode = if address == UNLOCK_ADDRESS_2 && value == 0x55 {
            FlashMode::Command
        } else {
            FlashMode::ReadArray
        };
    }

    fn write_command(&mut self, address: usize, value: u8) {
        if address != UNLOCK_ADDRESS_1 {
            return;
        }
        self.flash_mode = match value {
            0x90 => FlashMode::Id,
            0xA0 => FlashMode::Program(ProgramBuffer::new()),
            0x80 | 0x60 | 0x77 => FlashMode::FollowupFirst { command: value },
            _ => FlashMode::ReadArray,
        };
    }

    fn write_followup_first(&mut self, command: u8, address: usize, value: u8) {
        self.flash_mode = if address == UNLOCK_ADDRESS_1 && value == 0xAA {
            FlashMode::FollowupSecond { command }
        } else {
            FlashMode::ReadArray
        };
    }

    fn write_followup_second(&mut self, command: u8, address: usize, value: u8) {
        self.flash_mode = if address == UNLOCK_ADDRESS_2 && value == 0x55 {
            FlashMode::FollowupCommand { command }
        } else {
            FlashMode::ReadArray
        };
    }

    fn write_program(
        &mut self,
        mut buffer: ProgramBuffer,
        address: usize,
        value: u8,
        hidden: bool,
    ) {
        let storage_address = if hidden {
            address & (HIDDEN_SIZE - 1)
        } else {
            address
        };
        self.flash_mode = match buffer.write(storage_address, value) {
            ProgramAction::Collecting if hidden => FlashMode::HiddenProgram(buffer),
            ProgramAction::Collecting => FlashMode::Program(buffer),
            ProgramAction::Commit => FlashMode::Status {
                failed: !self.commit_program(&buffer, hidden),
            },
            ProgramAction::Failed => FlashMode::Status { failed: true },
        };
    }

    fn finish_followup(&mut self, command: u8, address: usize, value: u8) {
        match (command, value) {
            (0x80, 0x30) => {
                let sector = address / FLASH_SECTOR_SIZE;
                let failed = !self.erase_sector(sector);
                self.flash_mode = FlashMode::Status { failed };
            }
            (0x80, 0x10) if address == UNLOCK_ADDRESS_1 => {
                self.erase_chip();
                self.flash_mode = FlashMode::Status { failed: false };
            }
            (0x77, 0x77) if address == UNLOCK_ADDRESS_1 => {
                self.flash_mode = FlashMode::HiddenRead;
            }
            (0x60, 0xE0) if address == UNLOCK_ADDRESS_1 => {
                self.flash_mode = if self.flash_write_enabled {
                    FlashMode::HiddenProgram(ProgramBuffer::new())
                } else {
                    FlashMode::Status { failed: true }
                };
            }
            (0x60, 0x04) if address == UNLOCK_ADDRESS_1 => {
                let failed = !self.erase_hidden();
                self.flash_mode = FlashMode::Status { failed };
            }
            (0x60, 0x40) if address == UNLOCK_ADDRESS_1 => {
                let failed = !self.set_sector0_protected(false);
                self.flash_mode = FlashMode::Status { failed };
            }
            (0x60, 0x20) if address == UNLOCK_ADDRESS_1 => {
                let failed = !self.set_sector0_protected(true);
                self.flash_mode = FlashMode::Status { failed };
            }
            _ => self.flash_mode = FlashMode::ReadArray,
        }
    }

    fn commit_program(&mut self, buffer: &ProgramBuffer, hidden: bool) -> bool {
        let Some(start) = buffer.block_start else {
            return false;
        };
        if !buffer.written.iter().all(|written| *written) {
            return false;
        }
        if hidden {
            if !self.flash_write_enabled || start + PROGRAM_BLOCK_SIZE > self.hidden.len() {
                return false;
            }
            for (stored, programmed) in self.hidden[start..start + PROGRAM_BLOCK_SIZE]
                .iter_mut()
                .zip(&buffer.data)
            {
                *stored &= *programmed;
            }
            return true;
        }
        if start + PROGRAM_BLOCK_SIZE > self.flash.len()
            || (start < FLASH_SECTOR_SIZE && (!self.flash_write_enabled || self.sector0_protected))
        {
            return false;
        }
        for (stored, programmed) in self.flash[start..start + PROGRAM_BLOCK_SIZE]
            .iter_mut()
            .zip(&buffer.data)
        {
            *stored &= *programmed;
        }
        true
    }

    fn erase_sector(&mut self, sector: usize) -> bool {
        let start = sector.saturating_mul(FLASH_SECTOR_SIZE);
        if start >= self.flash.len()
            || (sector == 0 && (!self.flash_write_enabled || self.sector0_protected))
        {
            return false;
        }
        self.flash[start..start + FLASH_SECTOR_SIZE].fill(0xFF);
        true
    }

    fn erase_chip(&mut self) {
        if self.flash_write_enabled && !self.sector0_protected {
            self.flash[..FLASH_SECTOR_SIZE].fill(0xFF);
        }
        self.flash[FLASH_SECTOR_SIZE..].fill(0xFF);
    }

    fn erase_hidden(&mut self) -> bool {
        if !self.flash_write_enabled {
            return false;
        }
        self.hidden.fill(0xFF);
        true
    }

    fn set_sector0_protected(&mut self, protected: bool) -> bool {
        if !self.flash_write_enabled {
            return false;
        }
        self.sector0_protected = protected;
        true
    }

    fn machine_state_valid(state: &Mbc6MachineState) -> bool {
        state.sram.len() == SRAM_SIZE
            && state.flash.len() == FLASH_SIZE
            && state.hidden.len() == HIDDEN_SIZE
            && state.ram_bank_a <= 7
            && state.ram_bank_b <= 7
            && state.bank_a <= 0x7F
            && state.bank_b <= 0x7F
            && match &state.flash_mode {
                FlashMode::Program(buffer) => buffer.valid(FLASH_SIZE),
                FlashMode::HiddenProgram(buffer) => buffer.valid(HIDDEN_SIZE),
                _ => true,
            }
    }
}

impl Mbc for Mbc6 {
    fn kind(&self) -> MbcKind {
        MbcKind::Mbc6
    }

    fn read_rom0(&self, addr: u16) -> u8 {
        self.rom.get(usize::from(addr)).copied().unwrap_or(0xFF)
    }

    fn read_rom_n(&self, addr: u16) -> u8 {
        let (bank, flash) = self.selected_window(addr);
        let offset = Self::banked_offset(bank, addr);
        if flash {
            if self.flash_enabled {
                self.flash_read(offset)
            } else {
                0xFF
            }
        } else {
            self.rom.get(offset).copied().unwrap_or(0xFF)
        }
    }

    fn write_rom(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x03FF => self.ram_enabled = value == 0x0A,
            0x0400..=0x07FF => self.ram_bank_a = value & 0x07,
            0x0800..=0x0BFF => self.ram_bank_b = value & 0x07,
            0x0C00..=0x0FFF => self.flash_enabled = value & 1 != 0,
            0x1000 => self.flash_write_enabled = value & 1 != 0,
            0x2000..=0x27FF => self.bank_a = value & 0x7F,
            0x2800..=0x2FFF => self.flash_a = value == 0x08,
            0x3000..=0x37FF => self.bank_b = value & 0x7F,
            0x3800..=0x3FFF => self.flash_b = value == 0x08,
            0x4000..=0x7FFF => {
                let (bank, flash) = self.selected_window(addr);
                if flash && self.flash_enabled {
                    self.write_flash(Self::banked_offset(bank, addr), value);
                }
            }
            _ => {}
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        if !self.ram_enabled {
            return 0xFF;
        }
        let (bank, offset) = if addr < 0xB000 {
            (self.ram_bank_a, usize::from(addr - 0xA000))
        } else {
            (self.ram_bank_b, usize::from(addr - 0xB000))
        };
        self.sram[usize::from(bank) * RAM_BANK_SIZE + offset]
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram_enabled {
            return;
        }
        let (bank, offset) = if addr < 0xB000 {
            (self.ram_bank_a, usize::from(addr - 0xA000))
        } else {
            (self.ram_bank_b, usize::from(addr - 0xB000))
        };
        self.sram[usize::from(bank) * RAM_BANK_SIZE + offset] = value;
    }

    fn has_battery(&self) -> bool {
        true
    }

    fn ram_data(&self) -> Option<&[u8]> {
        Some(&self.sram)
    }

    fn ram_restore(&mut self, data: &[u8]) {
        if data.len() == self.sram.len() {
            self.sram.copy_from_slice(data);
        }
    }

    fn reset_runtime(&mut self) {
        self.ram_enabled = false;
        self.ram_bank_a = 0;
        self.ram_bank_b = 1;
        self.flash_enabled = false;
        self.flash_write_enabled = false;
        self.bank_a = 2;
        self.bank_b = 3;
        self.flash_a = false;
        self.flash_b = false;
        self.flash_mode = FlashMode::ReadArray;
    }

    fn export_persistent_state(&self, _now: SystemTime) -> Result<Option<Vec<u8>>, String> {
        rmp_serde::to_vec_named(&Mbc6PersistentState {
            schema_version: PERSISTENT_STATE_SCHEMA_VERSION,
            kind: self.kind(),
            sram: self.sram.clone(),
            flash: self.flash.clone(),
            hidden: self.hidden.clone(),
            sector0_protected: self.sector0_protected,
        })
        .map(Some)
        .map_err(|error| error.to_string())
    }

    fn import_persistent_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: Mbc6PersistentState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != PERSISTENT_STATE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported MBC6 persistent state version: {}",
                state.schema_version
            ));
        }
        if state.kind != self.kind() {
            return Err("MBC6 persistent state kind mismatch".into());
        }
        if state.sram.len() != SRAM_SIZE
            || state.flash.len() != FLASH_SIZE
            || state.hidden.len() != HIDDEN_SIZE
        {
            return Err("MBC6 persistent state length mismatch".into());
        }
        self.sram = state.sram;
        self.flash = state.flash;
        self.hidden = state.hidden;
        self.sector0_protected = state.sector0_protected;
        Ok(())
    }

    fn serialize_state(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(&Mbc6MachineState {
            schema_version: MACHINE_STATE_SCHEMA_VERSION,
            sram: self.sram.clone(),
            flash: self.flash.clone(),
            hidden: self.hidden.clone(),
            ram_enabled: self.ram_enabled,
            ram_bank_a: self.ram_bank_a,
            ram_bank_b: self.ram_bank_b,
            flash_enabled: self.flash_enabled,
            flash_write_enabled: self.flash_write_enabled,
            bank_a: self.bank_a,
            bank_b: self.bank_b,
            flash_a: self.flash_a,
            flash_b: self.flash_b,
            sector0_protected: self.sector0_protected,
            flash_mode: self.flash_mode.clone(),
        })
        .expect("MBC6 machine state should serialize")
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: Mbc6MachineState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != MACHINE_STATE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported MBC6 machine state version: {}",
                state.schema_version
            ));
        }
        if !Self::machine_state_valid(&state) {
            return Err("invalid MBC6 machine state".into());
        }
        self.sram = state.sram;
        self.flash = state.flash;
        self.hidden = state.hidden;
        self.ram_enabled = state.ram_enabled;
        self.ram_bank_a = state.ram_bank_a;
        self.ram_bank_b = state.ram_bank_b;
        self.flash_enabled = state.flash_enabled;
        self.flash_write_enabled = state.flash_write_enabled;
        self.bank_a = state.bank_a;
        self.bank_b = state.bank_b;
        self.flash_a = state.flash_a;
        self.flash_b = state.flash_b;
        self.sector0_protected = state.sector0_protected;
        self.flash_mode = state.flash_mode;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom() -> Vec<u8> {
        let mut rom = vec![0; 8 * ROM_BANK_SIZE];
        for bank in 0..8 {
            rom[bank * ROM_BANK_SIZE] = bank as u8;
        }
        rom
    }

    fn map_flash(mbc: &mut Mbc6) {
        mbc.write_rom(0x0C00, 1);
        mbc.write_rom(0x2800, 8);
    }

    fn select_flash_a(mbc: &mut Mbc6, bank: u8) {
        mbc.write_rom(0x2000, bank);
    }

    fn flash_write(mbc: &mut Mbc6, address: usize, value: u8) {
        select_flash_a(mbc, (address / ROM_BANK_SIZE) as u8);
        mbc.write_rom(0x4000 + (address % ROM_BANK_SIZE) as u16, value);
    }

    fn unlock_command(mbc: &mut Mbc6, command: u8) {
        flash_write(mbc, UNLOCK_ADDRESS_1, 0xAA);
        flash_write(mbc, UNLOCK_ADDRESS_2, 0x55);
        flash_write(mbc, UNLOCK_ADDRESS_1, command);
    }

    fn followup_command(mbc: &mut Mbc6, command: u8, address: usize, value: u8) {
        unlock_command(mbc, command);
        flash_write(mbc, UNLOCK_ADDRESS_1, 0xAA);
        flash_write(mbc, UNLOCK_ADDRESS_2, 0x55);
        flash_write(mbc, address, value);
    }

    #[test]
    fn defaults_and_switches_rom_windows_independently() {
        let mut mbc = Mbc6::new(rom());
        assert_eq!(mbc.read_rom_n(0x4000), 2);
        assert_eq!(mbc.read_rom_n(0x6000), 3);

        mbc.write_rom(0x2000, 4);
        mbc.write_rom(0x3000, 5);
        assert_eq!(mbc.read_rom_n(0x4000), 4);
        assert_eq!(mbc.read_rom_n(0x6000), 5);
    }

    #[test]
    fn switches_ram_windows_independently() {
        let mut mbc = Mbc6::new(rom());
        mbc.write_rom(0, 0x0A);
        mbc.write_rom(0x0400, 4);
        mbc.write_rom(0x0800, 5);
        mbc.write_ram(0xA000, 0x44);
        mbc.write_ram(0xB000, 0x55);
        assert_eq!(mbc.sram[4 * RAM_BANK_SIZE], 0x44);
        assert_eq!(mbc.sram[5 * RAM_BANK_SIZE], 0x55);
    }

    #[test]
    fn flash_id_mode_and_exit_work() {
        let mut mbc = Mbc6::new(rom());
        map_flash(&mut mbc);
        unlock_command(&mut mbc, 0x90);
        select_flash_a(&mut mbc, 0);
        assert_eq!(mbc.read_rom_n(0x4000), 0xC2);
        assert_eq!(mbc.read_rom_n(0x4001), 0x81);
        mbc.write_rom(0x4000, 0xF0);
        assert_eq!(mbc.read_rom_n(0x4000), 0xFF);
    }

    #[test]
    fn status_mode_requires_explicit_exit_before_another_command() {
        let mut mbc = Mbc6::new(rom());
        map_flash(&mut mbc);
        mbc.flash_mode = FlashMode::Status { failed: false };

        unlock_command(&mut mbc, 0x90);
        assert!(matches!(mbc.flash_mode, FlashMode::Status { .. }));

        flash_write(&mut mbc, 0, 0xF0);
        unlock_command(&mut mbc, 0x90);
        assert!(matches!(mbc.flash_mode, FlashMode::Id));
    }

    #[test]
    fn programs_complete_block_and_requires_sector0_write_enable() {
        let mut mbc = Mbc6::new(rom());
        map_flash(&mut mbc);
        mbc.write_rom(0x1000, 1);
        unlock_command(&mut mbc, 0xA0);
        for offset in 0..PROGRAM_BLOCK_SIZE {
            flash_write(&mut mbc, offset, offset as u8);
        }
        flash_write(&mut mbc, PROGRAM_BLOCK_SIZE - 1, 0);
        assert_eq!(
            &mbc.flash[..PROGRAM_BLOCK_SIZE],
            &(0..0x80).collect::<Vec<_>>()
        );
    }

    #[test]
    fn sector_and_chip_erase_respect_sector_zero_protection() {
        let mut mbc = Mbc6::new(rom());
        map_flash(&mut mbc);
        mbc.write_rom(0x1000, 1);
        mbc.flash[0] = 0;
        mbc.flash[FLASH_SECTOR_SIZE] = 0;
        followup_command(&mut mbc, 0x60, UNLOCK_ADDRESS_1, 0x20);
        assert!(mbc.sector0_protected);

        mbc.flash_mode = FlashMode::ReadArray;
        followup_command(&mut mbc, 0x80, UNLOCK_ADDRESS_1, 0x10);

        assert_eq!(mbc.flash[0], 0);
        assert_eq!(mbc.flash[FLASH_SECTOR_SIZE], 0xFF);
    }

    #[test]
    fn hidden_region_can_be_programmed_read_and_erased() {
        let mut mbc = Mbc6::new(rom());
        map_flash(&mut mbc);
        mbc.write_rom(0x1000, 1);
        followup_command(&mut mbc, 0x60, UNLOCK_ADDRESS_1, 0xE0);
        for offset in 0..PROGRAM_BLOCK_SIZE {
            flash_write(&mut mbc, offset, offset as u8);
        }
        flash_write(&mut mbc, PROGRAM_BLOCK_SIZE - 1, 0);
        assert_eq!(mbc.hidden[5], 5);

        mbc.flash_mode = FlashMode::ReadArray;
        followup_command(&mut mbc, 0x77, UNLOCK_ADDRESS_1, 0x77);
        assert!(matches!(mbc.flash_mode, FlashMode::HiddenRead));
        select_flash_a(&mut mbc, 0);
        assert_eq!(mbc.read_rom_n(0x4005), 5);

        mbc.flash_mode = FlashMode::ReadArray;
        followup_command(&mut mbc, 0x60, UNLOCK_ADDRESS_1, 0x04);
        assert!(mbc.hidden.iter().all(|byte| *byte == 0xFF));
    }

    #[test]
    fn protects_sector_zero_and_persists_all_nonvolatile_storage() {
        let mut source = Mbc6::new(rom());
        source.sram[1] = 1;
        source.flash[2] = 2;
        source.hidden[3] = 3;
        source.sector0_protected = true;
        let state = source
            .export_persistent_state(SystemTime::UNIX_EPOCH)
            .unwrap()
            .unwrap();
        let mut restored = Mbc6::new(rom());

        restored.import_persistent_state(&state).unwrap();

        assert_eq!(restored.sram[1], 1);
        assert_eq!(restored.flash[2], 2);
        assert_eq!(restored.hidden[3], 3);
        assert!(restored.sector0_protected);
    }

    #[test]
    fn machine_state_round_trips_command_progress() {
        let mut source = Mbc6::new(rom());
        map_flash(&mut source);
        flash_write(&mut source, UNLOCK_ADDRESS_1, 0xAA);
        let state = source.serialize_state();
        let mut restored = Mbc6::new(rom());
        restored.deserialize_state(&state).unwrap();

        flash_write(&mut restored, UNLOCK_ADDRESS_2, 0x55);
        flash_write(&mut restored, UNLOCK_ADDRESS_1, 0x90);
        select_flash_a(&mut restored, 0);
        assert_eq!(restored.read_rom_n(0x4000), 0xC2);
    }

    #[test]
    fn invalid_machine_state_is_transactional() {
        let source = Mbc6::new(rom());
        let mut state: Mbc6MachineState = rmp_serde::from_slice(&source.serialize_state()).unwrap();
        state.ram_bank_a = 8;
        let data = rmp_serde::to_vec_named(&state).unwrap();
        let mut target = Mbc6::new(rom());
        target.bank_a = 7;

        assert!(target.deserialize_state(&data).is_err());
        assert_eq!(target.bank_a, 7);
    }
}
