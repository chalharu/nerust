// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod error;
pub mod format;
pub mod mapper;
use self::format::CartridgeDataDao;
use crate::nes::MirrorMode;
use crate::nes::OpenBusReadResult;
use std::cmp;

pub trait Cartridge: Mapper {
    fn initialize(&mut self) {
        self.mapper_state_mut().has_battery =
            self.data_ref().has_battery() || self.battery_default();
        self.mapper_state_mut().sram = vec![
            0;
            cmp::max(
                self.data_ref().pram_length() + self.data_ref().save_pram_length(),
                self.ram_len_default()
            )
        ];
        if self.data_ref().char_rom_len() == 0 {
            self.mapper_state_mut().vram = vec![
                0;
                cmp::max(
                    self.data_ref().vram_length() + self.data_ref().save_vram_length(),
                    self.character_ram_page_len_default()
                )
            ];
            self.mapper_state_mut().character_mapping_mode = MappingMode::Ram;
        } else {
            self.mapper_state_mut().character_mapping_mode = MappingMode::Rom;
        };
        self.set_mirror_mode(self.data_ref().get_mirror_mode());
        Mapper::initialize(self);
    }

    fn read(&self, address: usize) -> OpenBusReadResult {
        match address {
            0...0x1FFF => self.read_character(address),
            0x6000...0x7FFF => Cartridge::read_ram(self, address - 0x6000),
            0x8000...0xFFFF => self.read_program(address - 0x8000),
            _ => {
                error!("unhandled mapper read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0xFF)
            }
        }
    }

    fn read_character(&self, address: usize) -> OpenBusReadResult {
        OpenBusReadResult::new(
            self.character_address(address).map_or_else(
                || {
                    self.character_openbus_default()
                        .unwrap_or((address & 0xFF) as u8)
                },
                |x| {
                    if self.mapper_state_ref().character_mapping_mode == MappingMode::Rom {
                        self.data_ref().read_char_rom(x)
                    } else {
                        self.mapper_state_ref().vram[x]
                    }
                },
            ),
            0xFF,
        )
    }

    fn read_ram(&self, address: usize) -> OpenBusReadResult {
        OpenBusReadResult::new(Mapper::read_ram(self, address).unwrap_or(0), 0xFF)
    }

    fn read_program(&self, address: usize) -> OpenBusReadResult {
        OpenBusReadResult::new(
            self.program_address(address)
                .map(|x| self.data_ref().read_prog_rom(x))
                .unwrap_or(0),
            0xFF,
        )
    }

    fn write(&mut self, address: usize, value: u8) {
        match address {
            0...0x1FFF => self.write_character(address, value),
            0x6000...0x7FFF => Cartridge::write_ram(self, address - 0x6000, value),
            0x8000...0xFFFF => self.write_program(address - 0x8000, value),
            _ => {
                error!("unhandled mapper write at address: 0x{:04X}", address);
            }
        }
    }

    fn write_character(&mut self, address: usize, value: u8) {
        if self.mapper_state_ref().character_mapping_mode == MappingMode::Ram {
            if let Some(addr) = self.character_address(address) {
                self.mapper_state_mut().vram[addr] = value;
            }
        }
    }

    fn write_ram(&mut self, address: usize, value: u8) {
        Mapper::write_ram(self, address, value);
    }

    fn write_program(&mut self, address: usize, value: u8) {
        if self.register_addr(address) {
            self.write_register(address, if self.bus_conflicts() {
                self.read_program(address).data & value
            } else {
                value
            });
        } else {
            if let Some(addr) = self.program_address(address) {
                self.data_mut().write_prog_rom(addr, value);
            }
        }
    }

    fn mirror_mode(&self) -> MirrorMode {
        self.get_mirror_mode()
    }
}

impl<T: Mapper> Cartridge for T {}

#[derive(Eq, PartialEq)]
enum MappingMode {
    Ram,
    Rom,
}

pub struct MapperState {
    program_page_table: [Option<usize>; 256],
    character_page_table: [Option<usize>; 256],
    sram_page_table: [Option<usize>; 256],
    sram: Vec<u8>,
    vram: Vec<u8>,
    mirror_mode: MirrorMode,
    has_battery: bool,
    character_mapping_mode: MappingMode,
}

impl MapperState {
    pub fn new() -> Self {
        Self {
            program_page_table: [None; 256],
            character_page_table: [None; 256],
            sram_page_table: [None; 256],
            sram: Vec::new(),
            vram: Vec::new(),
            mirror_mode: MirrorMode::try_from(0).unwrap(),
            has_battery: false,
            character_mapping_mode: MappingMode::Rom,
        }
    }
}

pub trait MapperStateDao {
    fn mapper_state_mut(&mut self) -> &mut MapperState;
    fn mapper_state_ref(&self) -> &MapperState;
}

pub trait Mapper: MapperStateDao + CartridgeDataDao {
    fn name(&self) -> &str;
    fn program_page_len(&self) -> usize;
    fn character_page_len(&self) -> usize;
    fn initialize(&mut self);
    fn ram_page_len(&self) -> usize {
        0x2000
    }

    #[allow(unused_variables)]
    fn register_addr(&self, address: usize) -> bool {
        true
    }

    #[allow(unused_variables)]
    fn write_register(&mut self, address: usize, value: u8) {}

    fn battery_default(&self) -> bool {
        false
    }

    fn save_len_default(&self) -> usize {
        if self.battery_default() {
            0x2000
        } else {
            0
        }
    }

    fn ram_len_default(&self) -> usize {
        if self.battery_default() {
            0x2000
        } else {
            0
        }
    }

    fn ram_page_len_default(&self) -> usize {
        if self.battery_default() {
            0x2000
        } else {
            0
        }
    }

    fn character_ram_page_len_default(&self) -> usize {
        0x2000
    }

    // fn character_ram_page_len(&self) -> usize;

    fn bus_conflicts(&self) -> bool {
        false
    }

    fn change_program_page(&mut self, offset: usize, page: usize) {
        let total_pages = self.data_ref().prog_rom_len() >> 8;
        let page_count = self.program_page_len() >> 8;
        let page_offset = offset * page_count;
        let mut page_value_offset = page * page_count;
        for i in page_offset..(page_offset + page_count) {
            while page_value_offset >= total_pages {
                page_value_offset -= total_pages;
            }
            self.mapper_state_mut().program_page_table[i] = Some(page_value_offset);
            page_value_offset += 1;
        }
    }

    fn change_character_page(&mut self, offset: usize, page: usize) {
        let total_pages = if self.mapper_state_ref().character_mapping_mode == MappingMode::Ram {
            self.mapper_state_ref().vram.len()
        } else {
            self.data_ref().char_rom_len()
        } >> 8;
        let page_count = self.character_page_len() >> 8;
        let page_offset = offset * page_count;
        let mut page_value_offset = page * page_count;
        for i in page_offset..(page_offset + page_count) {
            while page_value_offset >= total_pages {
                page_value_offset -= total_pages;
            }
            self.mapper_state_mut().character_page_table[i] = Some(page_value_offset);
            page_value_offset += 1;
        }
    }

    fn release_character_page(&mut self, offset: usize) {
        let page_count = self.character_page_len() >> 8;
        let page_offset = offset * page_count;
        for i in page_offset..(page_offset + page_count) {
            self.mapper_state_mut().character_page_table[i] = None;
        }
    }

    fn change_ram_page(&mut self, offset: usize, page: usize) {
        let total_pages = self.mapper_state_mut().sram.len() >> 8;
        let page_count = self.ram_page_len() >> 8;
        let page_offset = offset * page_count;
        let mut page_value_offset = page * page_count;
        for i in page_offset..(page_offset + page_count) {
            while page_value_offset >= total_pages {
                page_value_offset -= total_pages;
            }
            self.mapper_state_mut().sram_page_table[i] = Some(page_value_offset);
            page_value_offset += 1;
        }
    }

    fn program_address(&self, address: usize) -> Option<usize> {
        self.mapper_state_ref().program_page_table[address >> 8]
            .map(|x| (x << 8) | (address & 0xFF))
    }

    fn character_address(&self, address: usize) -> Option<usize> {
        self.mapper_state_ref().character_page_table[address >> 8]
            .map(|x| (x << 8) | (address & 0xFF))
    }

    fn ram_address(&self, address: usize) -> Option<usize> {
        self.mapper_state_ref().sram_page_table[address >> 8].map(|x| (x << 8) | (address & 0xFF))
    }

    fn character_openbus_default(&self) -> Option<u8> {
        None
    }

    fn get_mirror_mode(&self) -> MirrorMode {
        self.mapper_state_ref().mirror_mode
    }

    fn set_mirror_mode(&mut self, value: MirrorMode) {
        self.mapper_state_mut().mirror_mode = value;
    }

    fn read_ram(&self, index: usize) -> Option<u8> {
        self.ram_address(index)
            .map(|x| self.mapper_state_ref().sram[x])
    }

    fn write_ram(&mut self, index: usize, data: u8) {
        if let Some(addr) = self.ram_address(index) {
            self.mapper_state_mut().sram[addr] = data;
        }
    }

    fn step(&mut self) {}
}

pub fn try_from<I: Iterator<Item = u8>>(
    input: &mut I,
) -> Result<Box<Cartridge>, error::CartridgeError> {
    let mut result = mapper::try_from(format::CartridgeData::try_from(input)?);
    if let Ok(ref mut r) = result {
        Cartridge::initialize(r.as_mut());
    }
    result
}
