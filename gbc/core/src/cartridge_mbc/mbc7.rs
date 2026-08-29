use std::time::SystemTime;

use nerust_core_traits::peripheral::AccelerationSample;

use super::{Mbc, MbcKind};

const MACHINE_STATE_SCHEMA_VERSION: u32 = 1;
const PERSISTENT_STATE_SCHEMA_VERSION: u32 = 1;
const EEPROM_SIZE: usize = 0x100;
const EEPROM_WORDS: usize = EEPROM_SIZE / 2;
const COMMAND_BITS: u8 = 11;
const ACCELEROMETER_CENTER: i32 = 0x81D0;
const ACCELEROMETER_UNITS_PER_G: f32 = 0x70 as f32;
const MAX_ACCELERATION_G: f32 = 4.0;

#[derive(Debug, Clone)]
pub struct Mbc7 {
    rom: Vec<u8>,
    rom_bank: u8,
    ram_enable_1: bool,
    ram_enable_2: bool,
    latch_armed: bool,
    latched_x: u16,
    latched_y: u16,
    acceleration: Option<AccelerationSample>,
    eeprom: Eeprom,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Eeprom {
    #[serde(with = "serde_bytes")]
    data: Vec<u8>,
    write_enabled: bool,
    cs: bool,
    clk: bool,
    di: bool,
    do_: bool,
    phase: EepromPhase,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum EepromPhase {
    Idle,
    Command {
        bits: u16,
        count: u8,
    },
    Read {
        word: u16,
        bits_left: u8,
    },
    Write {
        address: u8,
        word: u16,
        bits_left: u8,
    },
    WriteAll {
        word: u16,
        bits_left: u8,
    },
    Ready,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Mbc7MachineState {
    schema_version: u32,
    rom_bank: u8,
    ram_enable_1: bool,
    ram_enable_2: bool,
    latch_armed: bool,
    latched_x: u16,
    latched_y: u16,
    eeprom: Eeprom,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Mbc7PersistentState {
    schema_version: u32,
    kind: MbcKind,
    #[serde(with = "serde_bytes")]
    eeprom: Vec<u8>,
}

impl Eeprom {
    fn new() -> Self {
        Self {
            data: vec![0xFF; EEPROM_SIZE],
            write_enabled: false,
            cs: false,
            clk: false,
            di: false,
            do_: true,
            phase: EepromPhase::Idle,
        }
    }

    fn read_pins(&self) -> u8 {
        u8::from(self.do_)
            | (u8::from(self.di) << 1)
            | (u8::from(self.clk) << 6)
            | (u8::from(self.cs) << 7)
    }

    fn write_pins(&mut self, value: u8) {
        let next_cs = value & 0x80 != 0;
        let next_clk = value & 0x40 != 0;
        let next_di = value & 0x02 != 0;
        if self.cs && !next_cs {
            self.phase = EepromPhase::Idle;
            self.do_ = true;
        }
        let rising_edge = next_cs && !self.clk && next_clk;
        self.cs = next_cs;
        self.clk = next_clk;
        self.di = next_di;
        if rising_edge {
            self.clock_rising_edge();
        }
    }

    fn clock_rising_edge(&mut self) {
        let phase = std::mem::replace(&mut self.phase, EepromPhase::Idle);
        match phase {
            EepromPhase::Idle => {
                if self.di {
                    self.phase = EepromPhase::Command { bits: 1, count: 1 };
                }
            }
            EepromPhase::Command {
                mut bits,
                mut count,
            } => {
                bits = (bits << 1) | u16::from(self.di);
                count += 1;
                if count == COMMAND_BITS {
                    self.decode_command(bits);
                } else {
                    self.phase = EepromPhase::Command { bits, count };
                }
            }
            EepromPhase::Read {
                mut word,
                mut bits_left,
            } => {
                self.do_ = word & 0x8000 != 0;
                word <<= 1;
                bits_left -= 1;
                self.phase = if bits_left == 0 {
                    EepromPhase::Ready
                } else {
                    EepromPhase::Read { word, bits_left }
                };
            }
            EepromPhase::Write {
                address,
                mut word,
                mut bits_left,
            } => {
                word = (word << 1) | u16::from(self.di);
                bits_left -= 1;
                if bits_left == 0 {
                    if self.write_enabled {
                        self.write_word(address, word);
                    }
                    self.do_ = true;
                    self.phase = EepromPhase::Ready;
                } else {
                    self.phase = EepromPhase::Write {
                        address,
                        word,
                        bits_left,
                    };
                }
            }
            EepromPhase::WriteAll {
                mut word,
                mut bits_left,
            } => {
                word = (word << 1) | u16::from(self.di);
                bits_left -= 1;
                if bits_left == 0 {
                    if self.write_enabled {
                        for address in 0..EEPROM_WORDS as u8 {
                            self.write_word(address, word);
                        }
                    }
                    self.do_ = true;
                    self.phase = EepromPhase::Ready;
                } else {
                    self.phase = EepromPhase::WriteAll { word, bits_left };
                }
            }
            EepromPhase::Ready => {
                self.do_ = true;
                self.phase = EepromPhase::Ready;
            }
        }
    }

    fn decode_command(&mut self, command: u16) {
        let opcode = (command >> 8) & 0x03;
        let address = (command & 0x7F) as u8;
        match opcode {
            0b10 => {
                self.phase = EepromPhase::Read {
                    word: self.read_word(address),
                    bits_left: 16,
                };
            }
            0b01 => {
                self.phase = EepromPhase::Write {
                    address,
                    word: 0,
                    bits_left: 16,
                };
            }
            0b11 => {
                if self.write_enabled {
                    self.write_word(address, 0xFFFF);
                }
                self.do_ = true;
                self.phase = EepromPhase::Ready;
            }
            0b00 => match (command >> 6) & 0x03 {
                0b00 => {
                    self.write_enabled = false;
                    self.phase = EepromPhase::Ready;
                }
                0b01 => {
                    self.phase = EepromPhase::WriteAll {
                        word: 0,
                        bits_left: 16,
                    };
                }
                0b10 => {
                    if self.write_enabled {
                        self.data.fill(0xFF);
                    }
                    self.do_ = true;
                    self.phase = EepromPhase::Ready;
                }
                0b11 => {
                    self.write_enabled = true;
                    self.phase = EepromPhase::Ready;
                }
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }

    fn read_word(&self, address: u8) -> u16 {
        let offset = usize::from(address) * 2;
        u16::from_be_bytes([self.data[offset], self.data[offset + 1]])
    }

    fn write_word(&mut self, address: u8, word: u16) {
        let offset = usize::from(address) * 2;
        self.data[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
    }

    fn reset_runtime(&mut self) {
        self.write_enabled = false;
        self.cs = false;
        self.clk = false;
        self.di = false;
        self.do_ = true;
        self.phase = EepromPhase::Idle;
    }

    fn valid(&self) -> bool {
        self.data.len() == EEPROM_SIZE
            && match self.phase {
                EepromPhase::Idle | EepromPhase::Ready => true,
                EepromPhase::Command { count, .. } => (1..COMMAND_BITS).contains(&count),
                EepromPhase::Read { bits_left, .. } => (1..=16).contains(&bits_left),
                EepromPhase::Write {
                    address, bits_left, ..
                } => usize::from(address) < EEPROM_WORDS && (1..=16).contains(&bits_left),
                EepromPhase::WriteAll { bits_left, .. } => (1..=16).contains(&bits_left),
            }
    }
}

impl Mbc7 {
    pub fn new(rom: Vec<u8>) -> Self {
        Self {
            rom,
            rom_bank: 1,
            ram_enable_1: false,
            ram_enable_2: false,
            latch_armed: false,
            latched_x: 0x8000,
            latched_y: 0x8000,
            acceleration: None,
            eeprom: Eeprom::new(),
        }
    }

    fn registers_enabled(&self) -> bool {
        self.ram_enable_1 && self.ram_enable_2
    }

    fn quantize_acceleration(value: f32) -> u16 {
        let value = if value.is_finite() {
            value.clamp(-MAX_ACCELERATION_G, MAX_ACCELERATION_G)
        } else {
            0.0
        };
        (ACCELEROMETER_CENTER + (value * ACCELEROMETER_UNITS_PER_G).round() as i32)
            .clamp(0, i32::from(u16::MAX)) as u16
    }

    fn latch_acceleration(&mut self) {
        let sample = self
            .acceleration
            .and_then(AccelerationSample::finite)
            .unwrap_or(AccelerationSample::new(0.0, 0.0));
        self.latched_x = Self::quantize_acceleration(sample.x_g);
        self.latched_y = Self::quantize_acceleration(sample.y_g);
    }
}

impl Mbc for Mbc7 {
    fn kind(&self) -> MbcKind {
        MbcKind::Mbc7
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
            0x0000..=0x1FFF => self.ram_enable_1 = value == 0x0A,
            0x2000..=0x3FFF => self.rom_bank = value & 0x7F,
            0x4000..=0x5FFF => self.ram_enable_2 = value == 0x40,
            _ => {}
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        if !self.registers_enabled() || addr >= 0xB000 {
            return 0xFF;
        }
        match (addr >> 4) & 0x0F {
            2 => self.latched_x as u8,
            3 => (self.latched_x >> 8) as u8,
            4 => self.latched_y as u8,
            5 => (self.latched_y >> 8) as u8,
            6 => 0,
            8 => self.eeprom.read_pins(),
            _ => 0xFF,
        }
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.registers_enabled() || addr >= 0xB000 {
            return;
        }
        match (addr >> 4) & 0x0F {
            0 if value == 0x55 => {
                self.latch_armed = true;
                self.latched_x = 0x8000;
                self.latched_y = 0x8000;
            }
            1 if value == 0xAA && self.latch_armed => {
                self.latch_acceleration();
                self.latch_armed = false;
            }
            8 => self.eeprom.write_pins(value),
            _ => {}
        }
    }

    fn has_battery(&self) -> bool {
        true
    }

    fn ram_data(&self) -> Option<&[u8]> {
        Some(&self.eeprom.data)
    }

    fn ram_restore(&mut self, data: &[u8]) {
        if data.len() == EEPROM_SIZE {
            self.eeprom.data.copy_from_slice(data);
        }
    }

    fn set_acceleration(&mut self, sample: Option<AccelerationSample>) {
        self.acceleration = sample.and_then(AccelerationSample::finite);
    }

    fn reset_runtime(&mut self) {
        self.rom_bank = 1;
        self.ram_enable_1 = false;
        self.ram_enable_2 = false;
        self.latch_armed = false;
        self.latched_x = 0x8000;
        self.latched_y = 0x8000;
        self.acceleration = None;
        self.eeprom.reset_runtime();
    }

    fn export_persistent_state(&self, _now: SystemTime) -> Result<Option<Vec<u8>>, String> {
        rmp_serde::to_vec_named(&Mbc7PersistentState {
            schema_version: PERSISTENT_STATE_SCHEMA_VERSION,
            kind: self.kind(),
            eeprom: self.eeprom.data.clone(),
        })
        .map(Some)
        .map_err(|error| error.to_string())
    }

    fn import_persistent_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: Mbc7PersistentState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != PERSISTENT_STATE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported MBC7 persistent state version: {}",
                state.schema_version
            ));
        }
        if state.kind != self.kind() {
            return Err("MBC7 persistent state kind mismatch".into());
        }
        if state.eeprom.len() != EEPROM_SIZE {
            return Err("MBC7 persistent EEPROM length mismatch".into());
        }
        self.eeprom.data = state.eeprom;
        Ok(())
    }

    fn serialize_state(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(&Mbc7MachineState {
            schema_version: MACHINE_STATE_SCHEMA_VERSION,
            rom_bank: self.rom_bank,
            ram_enable_1: self.ram_enable_1,
            ram_enable_2: self.ram_enable_2,
            latch_armed: self.latch_armed,
            latched_x: self.latched_x,
            latched_y: self.latched_y,
            eeprom: self.eeprom.clone(),
        })
        .expect("MBC7 machine state should serialize")
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: Mbc7MachineState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        if state.schema_version != MACHINE_STATE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported MBC7 machine state version: {}",
                state.schema_version
            ));
        }
        if state.rom_bank > 0x7F || !state.eeprom.valid() {
            return Err("invalid MBC7 machine state".into());
        }
        self.rom_bank = state.rom_bank;
        self.ram_enable_1 = state.ram_enable_1;
        self.ram_enable_2 = state.ram_enable_2;
        self.latch_armed = state.latch_armed;
        self.latched_x = state.latched_x;
        self.latched_y = state.latched_y;
        self.eeprom = state.eeprom;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CS: u8 = 0x80;
    const CLK: u8 = 0x40;
    const DI: u8 = 0x02;

    fn mbc() -> Mbc7 {
        let mut rom = vec![0; 0x20_0000];
        for bank in 0..128 {
            rom[bank * 0x4000] = bank as u8;
        }
        Mbc7::new(rom)
    }

    fn enable(mbc: &mut Mbc7) {
        mbc.write_rom(0, 0x0A);
        mbc.write_rom(0x4000, 0x40);
    }

    fn write_pins(mbc: &mut Mbc7, value: u8) {
        mbc.write_ram(0xA080, value);
    }

    fn clock_bit(mbc: &mut Mbc7, bit: bool) {
        let pins = CS | if bit { DI } else { 0 };
        write_pins(mbc, pins);
        write_pins(mbc, pins | CLK);
    }

    fn send_bits(mbc: &mut Mbc7, value: u16, count: u8) {
        for bit in (0..count).rev() {
            clock_bit(mbc, value & (1 << bit) != 0);
        }
    }

    fn begin(mbc: &mut Mbc7) {
        write_pins(mbc, 0);
        write_pins(mbc, CS);
    }

    fn end(mbc: &mut Mbc7) {
        write_pins(mbc, 0);
    }

    fn command(opcode: u16, address: u8) -> u16 {
        (1 << 10) | (opcode << 8) | u16::from(address)
    }

    fn special(subcommand: u16) -> u16 {
        (1 << 10) | (subcommand << 6)
    }

    fn send_command(mbc: &mut Mbc7, command: u16) {
        begin(mbc);
        send_bits(mbc, command, COMMAND_BITS);
    }

    fn set_write_enabled(mbc: &mut Mbc7, enabled: bool) {
        send_command(mbc, special(if enabled { 0b11 } else { 0b00 }));
        end(mbc);
    }

    fn write_word(mbc: &mut Mbc7, address: u8, word: u16) {
        send_command(mbc, command(0b01, address));
        send_bits(mbc, word, 16);
        end(mbc);
    }

    fn read_word(mbc: &mut Mbc7, address: u8) -> u16 {
        send_command(mbc, command(0b10, address));
        let mut word = 0;
        for _ in 0..16 {
            write_pins(mbc, CS);
            write_pins(mbc, CS | CLK);
            word = (word << 1) | u16::from(mbc.read_ram(0xA080) & 1);
        }
        end(mbc);
        word
    }

    #[test]
    fn switches_rom_bank_and_requires_both_register_enables() {
        let mut mbc = mbc();
        mbc.write_rom(0x2000, 127);
        assert_eq!(mbc.read_rom_n(0x4000), 127);
        assert_eq!(mbc.read_ram(0xA060), 0xFF);
        mbc.write_rom(0, 0x0A);
        assert_eq!(mbc.read_ram(0xA060), 0xFF);
        mbc.write_rom(0x4000, 0x40);
        assert_eq!(mbc.read_ram(0xA060), 0);
        assert_eq!(mbc.read_ram(0xB060), 0xFF);
    }

    #[test]
    fn latches_latest_acceleration_only_after_arm_sequence() {
        let mut mbc = mbc();
        enable(&mut mbc);
        mbc.set_acceleration(Some(AccelerationSample::new(1.0, -1.0)));
        mbc.write_ram(0xA010, 0xAA);
        assert_eq!(mbc.latched_x, 0x8000);
        mbc.write_ram(0xA000, 0x55);
        assert_eq!(mbc.latched_x, 0x8000);
        mbc.write_ram(0xA010, 0xAA);
        assert_eq!(mbc.latched_x, 0x8240);
        assert_eq!(mbc.latched_y, 0x8160);
    }

    #[test]
    fn unavailable_or_non_finite_acceleration_latches_center() {
        let mut mbc = mbc();
        enable(&mut mbc);
        mbc.set_acceleration(Some(AccelerationSample::new(f32::NAN, 1.0)));
        mbc.write_ram(0xA000, 0x55);
        mbc.write_ram(0xA010, 0xAA);
        assert_eq!(mbc.latched_x, ACCELEROMETER_CENTER as u16);
        assert_eq!(mbc.latched_y, ACCELEROMETER_CENTER as u16);
    }

    #[test]
    fn eeprom_write_read_erase_and_write_disable() {
        let mut mbc = mbc();
        enable(&mut mbc);
        write_word(&mut mbc, 5, 0x1234);
        assert_eq!(read_word(&mut mbc, 5), 0xFFFF);

        set_write_enabled(&mut mbc, true);
        write_word(&mut mbc, 5, 0x1234);
        assert_eq!(read_word(&mut mbc, 5), 0x1234);
        send_command(&mut mbc, command(0b11, 5));
        end(&mut mbc);
        assert_eq!(read_word(&mut mbc, 5), 0xFFFF);

        set_write_enabled(&mut mbc, false);
        write_word(&mut mbc, 5, 0xABCD);
        assert_eq!(read_word(&mut mbc, 5), 0xFFFF);
    }

    #[test]
    fn eeprom_write_all_and_erase_all_require_ewen() {
        let mut mbc = mbc();
        enable(&mut mbc);
        set_write_enabled(&mut mbc, true);
        send_command(&mut mbc, special(0b01));
        send_bits(&mut mbc, 0xA55A, 16);
        end(&mut mbc);
        assert_eq!(read_word(&mut mbc, 0), 0xA55A);
        assert_eq!(read_word(&mut mbc, 127), 0xA55A);

        send_command(&mut mbc, special(0b10));
        end(&mut mbc);
        assert!(mbc.eeprom.data.iter().all(|byte| *byte == 0xFF));
    }

    #[test]
    fn persistent_state_restores_only_eeprom_data() {
        let mut source = mbc();
        source.eeprom.write_word(3, 0xCAFE);
        source.eeprom.write_enabled = true;
        let state = source
            .export_persistent_state(SystemTime::UNIX_EPOCH)
            .unwrap()
            .unwrap();
        let mut restored = mbc();
        restored.import_persistent_state(&state).unwrap();

        assert_eq!(restored.eeprom.read_word(3), 0xCAFE);
        assert!(!restored.eeprom.write_enabled);
    }

    #[test]
    fn machine_state_round_trips_eeprom_command_progress() {
        let mut source = mbc();
        enable(&mut source);
        begin(&mut source);
        send_bits(&mut source, command(0b10, 7) >> 4, COMMAND_BITS - 4);
        let state = source.serialize_state();
        let mut restored = mbc();
        restored.deserialize_state(&state).unwrap();
        send_bits(&mut restored, command(0b10, 7) & 0x0F, 4);

        assert!(matches!(restored.eeprom.phase, EepromPhase::Read { .. }));
    }

    #[test]
    fn invalid_machine_state_is_transactional() {
        let source = mbc();
        let mut state: Mbc7MachineState = rmp_serde::from_slice(&source.serialize_state()).unwrap();
        state.eeprom.data.pop();
        let data = rmp_serde::to_vec_named(&state).unwrap();
        let mut target = mbc();
        target.rom_bank = 9;

        assert!(target.deserialize_state(&data).is_err());
        assert_eq!(target.rom_bank, 9);
    }
}
