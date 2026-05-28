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

    pub(crate) fn read(&self, header: &CartridgeHeader, address: u32) -> Option<u8> {
        match self {
            Self::None => None,
            Self::Sa1(state) => state.read(address),
            Self::SuperFx(state) => state.read(address),
            Self::Cx4(state) => state.read(address),
            Self::Dsp1(state) => state.read(header.mapper_kind(), address),
        }
    }

    pub(crate) fn write(&mut self, header: &CartridgeHeader, address: u32, value: u8) -> bool {
        match self {
            Self::None => false,
            Self::Sa1(state) => state.write(address, value),
            Self::SuperFx(state) => state.write(address, value),
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

impl SuperFxState {
    fn new() -> Self {
        Self {
            registers: ByteWindow::new(0x3000, 0x0300),
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
}

impl Dsp1State {
    fn new() -> Self {
        Self {
            data: 0,
            status: 0x80,
        }
    }

    fn read(&self, mapper_kind: MapperKind, address: u32) -> Option<u8> {
        let register_offset = dsp1_register_offset(mapper_kind, address)?;
        Some(if register_offset & 1 == 0 {
            self.data
        } else {
            self.status
        })
    }

    fn write(&mut self, mapper_kind: MapperKind, address: u32, value: u8) -> bool {
        if let Some(register_offset) = dsp1_register_offset(mapper_kind, address) {
            if register_offset & 1 == 0 {
                self.data = value;
            } else {
                self.status = value;
            }
            true
        } else {
            false
        }
    }
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
