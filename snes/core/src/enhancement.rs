use crate::cartridge::{CartridgeHeader, EnhancementChip};
use crate::mapper::MapperKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnhancementState {
    None,
    Sa1(Sa1State),
    SuperFx(SuperFxState),
    Cx4(Cx4State),
    Dsp1(Dsp1State),
}

impl EnhancementState {
    pub(crate) fn from_header(header: &CartridgeHeader) -> Self {
        match header.enhancement_chip() {
            EnhancementChip::None => Self::None,
            EnhancementChip::Sa1 => Self::Sa1(Sa1State::new()),
            EnhancementChip::SuperFxGsu1 | EnhancementChip::SuperFxGsu2 => {
                Self::SuperFx(SuperFxState::new())
            }
            EnhancementChip::Cx4 => Self::Cx4(Cx4State::new()),
            EnhancementChip::Dsp1Family => Self::Dsp1(Dsp1State::new()),
        }
    }

    pub(crate) fn peek(&self, header: &CartridgeHeader, address: u32) -> Option<u8> {
        match self {
            Self::None => None,
            Self::Sa1(state) => state.read(address),
            Self::SuperFx(state) => state.read(address),
            Self::Cx4(state) => state.read(address),
            Self::Dsp1(state) => state.peek(header.mapper_kind(), address),
        }
    }

    pub(crate) fn read(&mut self, header: &CartridgeHeader, address: u32) -> Option<u8> {
        match self {
            Self::None => None,
            Self::Sa1(state) => state.read(address),
            Self::SuperFx(state) => state.read(address),
            Self::Cx4(state) => state.read(address),
            Self::Dsp1(state) => state.read(header.mapper_kind(), address),
        }
    }

    pub(crate) fn write(
        &mut self,
        header: &CartridgeHeader,
        address: u32,
        value: u8,
        save_ram: &mut [u8],
    ) -> bool {
        match self {
            Self::None => false,
            Self::Sa1(state) => state.write(address, value),
            Self::SuperFx(state) => state.write(address, value, save_ram),
            Self::Cx4(state) => state.write(address, value),
            Self::Dsp1(state) => state.write(header.mapper_kind(), address, value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sa1State {
    registers: ByteWindow,
    iram: ByteWindow,
}

impl Sa1State {
    fn new() -> Self {
        Self {
            registers: ByteWindow::new(0x2200, 0x0200),
            iram: ByteWindow::new(0x3000, 0x0800),
        }
    }

    fn read(&self, address: u32) -> Option<u8> {
        if !is_system_bank(address) {
            return None;
        }

        self.registers
            .read(offset(address))
            .or_else(|| self.iram.read(offset(address)))
    }

    fn write(&mut self, address: u32, value: u8) -> bool {
        if !is_system_bank(address) {
            return false;
        }

        self.registers.write(offset(address), value) || self.iram.write(offset(address), value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SuperFxState {
    registers: ByteWindow,
}

const SUPERFX_VCR: u16 = 0x303B;
const SUPERFX_SFR: u16 = 0x3030;
const SUPERFX_R15: u16 = 0x301E;
const SUPERFX_R15_HIGH: u16 = 0x301F;
const SUPERFX_SCBR: u16 = 0x3038;
const SUPERFX_GO_FLAG: u8 = 0x20;

impl SuperFxState {
    fn new() -> Self {
        let mut registers = ByteWindow::new(0x3000, 0x0500);
        registers.write(SUPERFX_VCR, 0x04);
        Self { registers }
    }

    fn read(&self, address: u32) -> Option<u8> {
        if is_system_bank(address) {
            self.registers.read(offset(address))
        } else {
            None
        }
    }

    fn write(&mut self, address: u32, value: u8, save_ram: &mut [u8]) -> bool {
        if !is_system_bank(address) {
            return false;
        }

        let address_offset = offset(address);
        if address_offset == SUPERFX_VCR {
            return self.registers.contains(address_offset);
        }

        let handled = self.registers.write(address_offset, value);
        if handled && address_offset == SUPERFX_R15_HIGH {
            self.run_program(save_ram);
        }
        handled
    }

    fn run_program(&mut self, save_ram: &mut [u8]) {
        let r15 = u16::from_le_bytes([
            self.registers.read(SUPERFX_R15).unwrap_or(0),
            self.registers.read(SUPERFX_R15 + 1).unwrap_or(0),
        ]);
        let screen_base = usize::from(self.registers.read(SUPERFX_SCBR).unwrap_or(0)) * 0x400;
        GsuInterpreter::new(r15, screen_base, save_ram).run();

        let sfr = self.registers.read(SUPERFX_SFR).unwrap_or(0) & !SUPERFX_GO_FLAG;
        self.registers.write(SUPERFX_SFR, sfr);
    }
}

struct GsuInterpreter<'a> {
    ram: &'a mut [u8],
    registers: [u16; 16],
    pc: u16,
    source: usize,
    destination: Option<usize>,
    color: u8,
    zero: bool,
    screen_base: usize,
    halted: bool,
}

impl<'a> GsuInterpreter<'a> {
    fn new(entry: u16, screen_base: usize, ram: &'a mut [u8]) -> Self {
        let mut registers = [0; 16];
        registers[15] = entry;
        Self {
            ram,
            registers,
            pc: entry,
            source: 0,
            destination: None,
            color: 0,
            zero: false,
            screen_base,
            halted: false,
        }
    }

    fn run(&mut self) {
        for _ in 0..4096 {
            if self.halted {
                break;
            }
            if !self.step() {
                break;
            }
        }
    }

    fn step(&mut self) -> bool {
        let opcode = self.fetch();
        match opcode {
            0x00 => {
                self.sync_program_counter();
                self.halted = true;
            }
            0x01 | 0x02 => self.sync_program_counter(),
            0x08 => {
                let relative = self.fetch() as i8;
                self.sync_program_counter();
                if !self.zero {
                    self.branch(relative);
                }
            }
            0x20..=0x2F => {
                let register = usize::from(opcode & 0x0F);
                if self.read_ram(self.pc) & 0xF0 == 0xB0 {
                    let operand = self.fetch();
                    self.sync_program_counter();
                    let source = usize::from(operand & 0x0F);
                    let value = self.registers[source];
                    self.set_register(register, value);
                } else {
                    self.sync_program_counter();
                    self.source = register;
                    self.destination = Some(register);
                }
            }
            0x3C => {
                self.sync_program_counter();
                self.registers[12] = self.registers[12].wrapping_sub(1);
                self.zero = self.registers[12] == 0;
                if !self.zero {
                    self.pc = self.registers[13];
                    self.registers[15] = self.pc;
                }
            }
            0x3F => {
                let operand = self.fetch();
                self.sync_program_counter();
                if operand & 0xF0 == 0x60 {
                    self.compare_register(usize::from(operand & 0x0F));
                } else {
                    return false;
                }
            }
            0x30..=0x3F => {
                self.sync_program_counter();
                self.store_word(usize::from(opcode & 0x0F));
            }
            0x4C => {
                self.sync_program_counter();
                self.plot();
            }
            0x4E => {
                self.sync_program_counter();
                self.color = self.registers[0] as u8 & 0x0F;
            }
            0x50..=0x5F => {
                self.sync_program_counter();
                self.add_register(usize::from(opcode & 0x0F));
            }
            0xA0..=0xAF => {
                let value = self.fetch();
                self.sync_program_counter();
                self.set_register(usize::from(opcode & 0x0F), u16::from(value));
            }
            0xB0..=0xBF => {
                self.sync_program_counter();
                self.source = usize::from(opcode & 0x0F);
                self.destination = None;
            }
            0xD0..=0xDF => {
                self.sync_program_counter();
                let register = usize::from(opcode & 0x0F);
                self.registers[register] = self.registers[register].wrapping_add(1);
                self.zero = self.registers[register] == 0;
                self.source = register;
            }
            0xF0..=0xFF => {
                let low = self.fetch();
                let high = self.fetch();
                self.sync_program_counter();
                self.set_register(usize::from(opcode & 0x0F), u16::from_le_bytes([low, high]));
            }
            _ => return false,
        }
        true
    }

    fn fetch(&mut self) -> u8 {
        let value = self.read_ram(self.pc);
        self.pc = self.pc.wrapping_add(1);
        value
    }

    fn sync_program_counter(&mut self) {
        self.registers[15] = self.pc;
    }

    fn branch(&mut self, relative: i8) {
        self.pc = self.pc.wrapping_add_signed(i16::from(relative));
        self.registers[15] = self.pc;
    }

    fn set_register(&mut self, register: usize, value: u16) {
        self.registers[register] = value;
        self.zero = value == 0;
        self.source = register;
        self.destination = None;
    }

    fn compare_register(&mut self, register: usize) {
        self.zero = self.registers[self.source] == self.registers[register];
        self.destination = None;
    }

    fn add_register(&mut self, register: usize) {
        let result = self.registers[self.source].wrapping_add(self.registers[register]);
        let destination = self.destination.take().unwrap_or(0);
        self.set_register(destination, result);
    }

    fn store_word(&mut self, register: usize) {
        let address = self.registers[register];
        let value = self.registers[self.source].to_le_bytes();
        self.write_ram(address, value[0]);
        self.write_ram(address.wrapping_add(1), value[1]);
        self.destination = None;
    }

    fn plot(&mut self) {
        let x = usize::from(self.registers[1]);
        let y = usize::from(self.registers[2]);
        let tile_index = (y / 8) * 16 + (x / 8);
        let tile_base = self.screen_base + tile_index * 32;
        let row = y & 0x07;
        let bit = 0x80 >> (x & 0x07);
        for plane in 0..4 {
            let byte_offset = tile_base + row * 2 + (plane & 0x01) + (plane / 2) * 16;
            let mut value = self.read_ram_usize(byte_offset);
            if self.color & (1 << plane) != 0 {
                value |= bit;
            } else {
                value &= !bit;
            }
            self.write_ram_usize(byte_offset, value);
        }
        self.registers[1] = self.registers[1].wrapping_add(1);
        self.source = 1;
        self.destination = None;
    }

    fn read_ram(&self, address: u16) -> u8 {
        self.read_ram_usize(usize::from(address))
    }

    fn read_ram_usize(&self, address: usize) -> u8 {
        if self.ram.is_empty() {
            0
        } else {
            self.ram[address % self.ram.len()]
        }
    }

    fn write_ram(&mut self, address: u16, value: u8) {
        self.write_ram_usize(usize::from(address), value);
    }

    fn write_ram_usize(&mut self, address: usize, value: u8) {
        if !self.ram.is_empty() {
            self.ram[address % self.ram.len()] = value;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cx4State {
    registers: ByteWindow,
}

impl Cx4State {
    fn new() -> Self {
        Self {
            registers: ByteWindow::new(0x7F40, 0x0070),
        }
    }

    fn read(&self, address: u32) -> Option<u8> {
        if is_system_bank(address) {
            self.registers.read(offset(address))
        } else {
            None
        }
    }

    fn write(&mut self, address: u32, value: u8) -> bool {
        is_system_bank(address) && self.registers.write(offset(address), value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dsp1State {
    data: u8,
    status: u8,
    phase: Dsp1Phase,
    command: u8,
    expected_input_words: usize,
    input_low_byte: u8,
    input_words: Vec<u16>,
    output_words: Vec<u16>,
    output_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dsp1Phase {
    WaitingCommand,
    ReadingData,
    WritingData,
    Frozen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dsp1Operation {
    Multiply,
    Multiply2,
    MemoryTest,
    MemorySize,
    Radius,
    Range,
    Range2,
    MemoryDump,
    Unsupported,
    Freeze,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Dsp1CommandSpec {
    reads: usize,
    writes: usize,
    operation: Dsp1Operation,
}

const DSP1_STATUS_DRC: u8 = 0x04;
const DSP1_STATUS_DRS: u8 = 0x10;
const DSP1_STATUS_RQM: u8 = 0x80;
const DSP1_RESET_STATUS: u8 = DSP1_STATUS_DRC | DSP1_STATUS_RQM;

impl Dsp1State {
    fn new() -> Self {
        Self {
            data: 0,
            status: DSP1_RESET_STATUS,
            phase: Dsp1Phase::WaitingCommand,
            command: 0,
            expected_input_words: 0,
            input_low_byte: 0,
            input_words: Vec::new(),
            output_words: Vec::new(),
            output_index: 0,
        }
    }

    fn peek(&self, mapper_kind: MapperKind, address: u32) -> Option<u8> {
        let register_offset = dsp1_register_offset(mapper_kind, address)?;
        Some(if register_offset & 1 == 0 {
            self.peek_data()
        } else {
            self.status
        })
    }

    fn read(&mut self, mapper_kind: MapperKind, address: u32) -> Option<u8> {
        let register_offset = dsp1_register_offset(mapper_kind, address)?;
        Some(if register_offset & 1 == 0 {
            self.read_data()
        } else {
            self.status
        })
    }

    fn write(&mut self, mapper_kind: MapperKind, address: u32, value: u8) -> bool {
        if let Some(register_offset) = dsp1_register_offset(mapper_kind, address) {
            if register_offset & 1 == 0 {
                self.write_data(value);
            }
            true
        } else {
            false
        }
    }

    fn peek_data(&self) -> u8 {
        if self.phase != Dsp1Phase::WritingData {
            return self.data;
        }

        let word = self
            .output_words
            .get(self.output_index)
            .copied()
            .unwrap_or(0x0080);
        if self.status & DSP1_STATUS_DRS == 0 {
            word as u8
        } else {
            (word >> 8) as u8
        }
    }

    fn read_data(&mut self) -> u8 {
        if self.phase != Dsp1Phase::WritingData {
            return self.data;
        }

        let word = self
            .output_words
            .get(self.output_index)
            .copied()
            .unwrap_or(0x0080);
        if self.status & DSP1_STATUS_DRS == 0 {
            self.status |= DSP1_STATUS_DRS;
            let value = word as u8;
            self.data = value;
            return value;
        }

        self.status &= !DSP1_STATUS_DRS;
        let value = (word >> 8) as u8;
        self.data = value;
        self.output_index += 1;
        if self.output_index >= self.output_words.len() {
            self.finish_command();
        }
        value
    }

    fn write_data(&mut self, value: u8) {
        self.data = value;
        match self.phase {
            Dsp1Phase::WaitingCommand => self.start_command(value),
            Dsp1Phase::ReadingData => self.write_input_byte(value),
            Dsp1Phase::WritingData | Dsp1Phase::Frozen => {}
        }
    }

    fn start_command(&mut self, value: u8) {
        if value & 0xC0 != 0 {
            return;
        }

        self.command = value & 0x3F;
        let spec = dsp1_command_spec(self.command);
        if spec.operation == Dsp1Operation::Freeze {
            self.freeze();
            return;
        }

        self.status = DSP1_STATUS_RQM;
        self.expected_input_words = spec.reads;
        self.input_low_byte = 0;
        self.input_words.clear();
        self.output_words.clear();
        self.output_index = 0;
        if spec.reads == 0 {
            self.execute_command(spec);
        } else {
            self.phase = Dsp1Phase::ReadingData;
        }
    }

    fn write_input_byte(&mut self, value: u8) {
        if self.status & DSP1_STATUS_DRS == 0 {
            self.input_low_byte = value;
            self.status |= DSP1_STATUS_DRS;
            return;
        }

        self.status &= !DSP1_STATUS_DRS;
        self.input_words
            .push(u16::from_le_bytes([self.input_low_byte, value]));
        if self.input_words.len() >= self.expected_input_words {
            self.execute_command(dsp1_command_spec(self.command));
        }
    }

    fn execute_command(&mut self, spec: Dsp1CommandSpec) {
        self.output_words = match spec.operation {
            Dsp1Operation::Multiply => {
                vec![dsp1_multiply(self.input_words[0], self.input_words[1], 0)]
            }
            Dsp1Operation::Multiply2 => {
                vec![dsp1_multiply(self.input_words[0], self.input_words[1], 1)]
            }
            Dsp1Operation::MemoryTest => vec![0x0000],
            Dsp1Operation::MemorySize => vec![0x0100],
            Dsp1Operation::Radius => dsp1_radius(&self.input_words),
            Dsp1Operation::Range => vec![dsp1_range(&self.input_words, 0)],
            Dsp1Operation::Range2 => vec![dsp1_range(&self.input_words, 1)],
            Dsp1Operation::MemoryDump => vec![0; spec.writes],
            Dsp1Operation::Unsupported => vec![0; spec.writes],
            Dsp1Operation::Freeze => {
                self.freeze();
                return;
            }
        };
        self.output_index = 0;
        self.status &= !DSP1_STATUS_DRS;
        if self.output_words.is_empty() {
            self.finish_command();
        } else {
            self.phase = Dsp1Phase::WritingData;
        }
    }

    fn finish_command(&mut self) {
        self.data = 0x80;
        self.status = DSP1_RESET_STATUS;
        self.phase = Dsp1Phase::WaitingCommand;
        self.expected_input_words = 0;
        self.input_words.clear();
        self.output_words.clear();
        self.output_index = 0;
    }

    fn freeze(&mut self) {
        self.status &= !DSP1_STATUS_RQM;
        self.status &= !DSP1_STATUS_DRS;
        self.phase = Dsp1Phase::Frozen;
    }
}

fn dsp1_command_spec(command: u8) -> Dsp1CommandSpec {
    use Dsp1Operation as Op;

    let (reads, writes, operation) = match command {
        0x00 => (2, 1, Op::Multiply),
        0x01 | 0x05 | 0x11 | 0x15 | 0x21 | 0x25 | 0x31 | 0x35 => (4, 0, Op::Unsupported),
        0x02 | 0x12 | 0x22 | 0x32 => (7, 4, Op::Unsupported),
        0x03 | 0x13 | 0x23 | 0x33 => (3, 3, Op::Unsupported),
        0x04 | 0x24 => (2, 2, Op::Unsupported),
        0x06 | 0x16 | 0x26 | 0x36 => (3, 3, Op::Unsupported),
        0x07 | 0x09 | 0x17 | 0x1A | 0x27 | 0x2A | 0x37 | 0x3A => (0, 0, Op::Freeze),
        0x08 => (3, 2, Op::Radius),
        0x0A => (1, 4, Op::Unsupported),
        0x0B | 0x1B => (3, 1, Op::Unsupported),
        0x0C | 0x2C => (3, 2, Op::Unsupported),
        0x0D | 0x1D | 0x2D | 0x3D => (3, 3, Op::Unsupported),
        0x0E | 0x2E => (2, 2, Op::Unsupported),
        0x0F => (1, 1, Op::MemoryTest),
        0x10 | 0x30 => (2, 2, Op::Unsupported),
        0x14 => (6, 3, Op::Unsupported),
        0x18 => (4, 1, Op::Range),
        0x1F | 0x3F => (1, 1024, Op::MemoryDump),
        0x20 => (2, 1, Op::Multiply2),
        0x28 => (3, 1, Op::Unsupported),
        0x2F => (1, 1, Op::MemorySize),
        0x38 => (4, 1, Op::Range2),
        _ => (0, 0, Op::Unsupported),
    };

    Dsp1CommandSpec {
        reads,
        writes,
        operation,
    }
}

fn dsp1_multiply(left: u16, right: u16, round: i32) -> u16 {
    let product = i32::from(left as i16) * i32::from(right as i16);
    ((product >> 15) + round) as i16 as u16
}

fn dsp1_radius(input_words: &[u16]) -> Vec<u16> {
    let sum = input_words
        .iter()
        .take(3)
        .map(|value| {
            let value = i64::from(*value as i16);
            value * value
        })
        .sum::<i64>() as u32;
    vec![sum as u16, (sum >> 16) as u16]
}

fn dsp1_range(input_words: &[u16], round: i64) -> u16 {
    let sum = input_words
        .iter()
        .take(3)
        .map(|value| {
            let value = i64::from(*value as i16);
            value * value
        })
        .sum::<i64>();
    let radius = i64::from(input_words.get(3).copied().unwrap_or(0) as i16);
    (((sum - radius * radius) >> 15) + round) as i16 as u16
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ByteWindow {
    start: u16,
    bytes: Vec<u8>,
}

impl ByteWindow {
    fn new(start: u16, len: usize) -> Self {
        Self {
            start,
            bytes: vec![0; len],
        }
    }

    fn read(&self, address_offset: u16) -> Option<u8> {
        self.index(address_offset).map(|index| self.bytes[index])
    }

    fn write(&mut self, address_offset: u16, value: u8) -> bool {
        if let Some(index) = self.index(address_offset) {
            self.bytes[index] = value;
            true
        } else {
            false
        }
    }

    fn contains(&self, address_offset: u16) -> bool {
        self.index(address_offset).is_some()
    }

    fn index(&self, address_offset: u16) -> Option<usize> {
        let relative = address_offset.checked_sub(self.start)? as usize;
        (relative < self.bytes.len()).then_some(relative)
    }
}

fn dsp1_register_offset(mapper_kind: MapperKind, address: u32) -> Option<u16> {
    let bank = bank(address);
    let offset = offset(address);

    match mapper_kind {
        MapperKind::LoRom => {
            if matches!(bank, 0x20..=0x3F | 0xA0..=0xBF) && offset >= 0x8000 {
                Some(offset - 0x8000)
            } else {
                None
            }
        }
        MapperKind::HiRom => {
            if matches!(bank, 0x00..=0x1F | 0x80..=0x9F) && (0x6000..=0x7FFF).contains(&offset) {
                Some(offset - 0x6000)
            } else {
                None
            }
        }
        MapperKind::Sa1 => None,
    }
}

fn is_system_bank(address: u32) -> bool {
    matches!(bank(address), 0x00..=0x3F | 0x80..=0xBF)
}

fn bank(address: u32) -> u8 {
    ((address >> 16) & 0xFF) as u8
}

fn offset(address: u32) -> u16 {
    (address & 0xFFFF) as u16
}
