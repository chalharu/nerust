// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::CartridgeData;
use crate::OpenBusReadResult;
use crate::cart_device::Cartridge;
use crate::cpu::interrupt::Interrupt;
use crate::mapper::{CartridgeDataDao, Mapper};
use crate::mapper_state::{MapperState, MapperStateDao};
use crate::ppu_memory_access::PpuReadAccess;

mod ppu;
mod program;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, serde_derive::Serialize, serde_derive::Deserialize, PartialEq, Eq)]
enum ChrBankSet {
    Sprite,
    Background,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Mmc5 {
    cartridge_data: CartridgeData,
    state: MapperState,
    prg_mode: u8,
    chr_mode: u8,
    prg_ram_protect_1: u8,
    prg_ram_protect_2: u8,
    exram_mode: u8,
    nametable_mapping: [u8; 4],
    fill_tile: u8,
    fill_attribute: u8,
    prg_banks: [u8; 5],
    sprite_chr_banks: [u16; 8],
    background_chr_banks: [u16; 4],
    chr_upper_bits: u8,
    sprite_size_16: bool,
    substitutions_enabled: bool,
    last_chr_bank_set: ChrBankSet,
    current_background_tile_index: usize,
    exram: Vec<u8>,
}

#[typetag::serde]
impl Cartridge for Mmc5 {
    fn read_character(&self, address: usize) -> OpenBusReadResult {
        self.read_character_with_access(address, PpuReadAccess::CpuData)
    }

    fn write_character(&mut self, address: usize, value: u8) {
        self.write_character_with_access(address, value, PpuReadAccess::CpuData);
    }

    fn read_ram(&self, address: usize) -> OpenBusReadResult {
        self.read_program_target(self.program_target_6000_7fff(address + 0x6000))
    }

    fn write_ram(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.write_program_target(self.program_target_6000_7fff(address + 0x6000), value);
    }

    fn read_program(&self, address: usize) -> OpenBusReadResult {
        self.read_program_target(self.program_target_8000_ffff(address + 0x8000))
    }

    fn write_program(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        let cpu_address = address + 0x8000;
        self.write_program_target(self.program_target_8000_ffff(cpu_address), value);
    }

    fn notify_ppu_ctrl(&mut self, value: u8) {
        self.sprite_size_16 = value & 0x20 != 0;
    }

    fn notify_ppu_mask(&mut self, value: u8) {
        self.substitutions_enabled = value & 0x18 != 0;
    }

    fn read_ppu_pattern(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        _interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        self.read_character_with_access(address, access)
    }

    fn write_ppu_pattern(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.write_character_with_access(address, value, PpuReadAccess::CpuData);
    }

    fn read_ppu_nametable(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        ciram: &mut [u8],
    ) -> OpenBusReadResult {
        let (table, offset) = Self::nametable_table_and_offset(address);
        if matches!(access, PpuReadAccess::BackgroundNameTable) {
            self.current_background_tile_index = offset & 0x03FF;
        }
        if matches!(access, PpuReadAccess::BackgroundAttribute)
            && self.extended_attributes_enabled()
        {
            return OpenBusReadResult::new(self.extended_attribute_byte(), 0xFF);
        }

        match self.nametable_mapping[table] {
            0 | 1 => OpenBusReadResult::new(
                ciram[(usize::from(self.nametable_mapping[table] & 0x01) << 10) | offset],
                0xFF,
            ),
            2 => {
                if self.exram_visible_to_ppu() {
                    OpenBusReadResult::new(self.exram[offset], 0xFF)
                } else {
                    OpenBusReadResult::new(0, 0xFF)
                }
            }
            3 => OpenBusReadResult::new(
                if offset >= 0x03C0 {
                    self.fill_attribute_byte()
                } else {
                    self.fill_tile
                },
                0xFF,
            ),
            _ => unreachable!(),
        }
    }

    fn write_ppu_nametable(
        &mut self,
        address: usize,
        value: u8,
        ciram: &mut [u8],
        _interrupt: &mut Interrupt,
    ) {
        let (table, offset) = Self::nametable_table_and_offset(address);
        match self.nametable_mapping[table] {
            0 | 1 => {
                let page = usize::from(self.nametable_mapping[table] & 0x01);
                ciram[(page << 10) | offset] = value;
            }
            2 if self.exram_visible_to_ppu() => self.exram[offset] = value,
            _ => {}
        }
    }

    fn peek_ppu_nametable(&self, address: usize, ciram: &[u8]) -> Option<u8> {
        let (table, offset) = Self::nametable_table_and_offset(address);
        Some(match self.nametable_mapping[table] {
            0 | 1 => {
                let page = usize::from(self.nametable_mapping[table] & 0x01);
                ciram[(page << 10) | offset]
            }
            2 => {
                if self.exram_visible_to_ppu() {
                    self.exram[offset]
                } else {
                    0
                }
            }
            3 => {
                if offset >= 0x03C0 {
                    self.fill_attribute_byte()
                } else {
                    self.fill_tile
                }
            }
            _ => unreachable!(),
        })
    }
}

impl Mmc5 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            prg_mode: 3,
            chr_mode: 0,
            prg_ram_protect_1: 0x03,
            prg_ram_protect_2: 0x03,
            exram_mode: 0x03,
            nametable_mapping: [0, 0, 1, 1],
            fill_tile: 0,
            fill_attribute: 0,
            prg_banks: [0, 0, 0, 0, 0xFF],
            sprite_chr_banks: [0; 8],
            background_chr_banks: [0; 4],
            chr_upper_bits: 0,
            sprite_size_16: false,
            substitutions_enabled: false,
            last_chr_bank_set: ChrBankSet::Sprite,
            current_background_tile_index: 0,
            exram: vec![0; 0x400],
        }
    }

    fn expand_chr_bank(&self, value: u8) -> u16 {
        u16::from(value) | (u16::from(self.chr_upper_bits & 0x03) << 8)
    }
}

impl CartridgeDataDao for Mmc5 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }

    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for Mmc5 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }

    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for Mmc5 {
    fn program_page_len(&self) -> usize {
        0x2000
    }

    fn character_page_len(&self) -> usize {
        0x0400
    }

    fn ram_len_default(&self) -> usize {
        0x10000
    }

    fn initialize(&mut self) {
        self.set_mirror_mode(match self.data_ref().mirror_mode() {
            crate::MirrorMode::Vertical => crate::MirrorMode::Vertical,
            crate::MirrorMode::Horizontal => crate::MirrorMode::Horizontal,
            mode => mode,
        });
    }

    fn name(&self) -> &str {
        "MMC5 (Mapper5)"
    }

    fn read_expansion(&self, address: usize) -> OpenBusReadResult {
        match address {
            0x5C00..=0x5FFF => self.read_exram_cpu(address),
            _ => OpenBusReadResult::new(0, 0),
        }
    }

    fn write_expansion(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        match address {
            0x5100 => self.prg_mode = value & 0x03,
            0x5101 => self.chr_mode = value & 0x03,
            0x5102 => self.prg_ram_protect_1 = value & 0x03,
            0x5103 => self.prg_ram_protect_2 = value & 0x03,
            0x5104 => self.exram_mode = value & 0x03,
            0x5105 => {
                self.nametable_mapping = [
                    value & 0x03,
                    (value >> 2) & 0x03,
                    (value >> 4) & 0x03,
                    (value >> 6) & 0x03,
                ];
            }
            0x5106 => self.fill_tile = value,
            0x5107 => self.fill_attribute = value & 0x03,
            0x5113..=0x5117 => self.prg_banks[address - 0x5113] = value,
            0x5120..=0x5127 => {
                self.sprite_chr_banks[address - 0x5120] = self.expand_chr_bank(value);
                self.last_chr_bank_set = ChrBankSet::Sprite;
            }
            0x5128..=0x512B => {
                self.background_chr_banks[address - 0x5128] = self.expand_chr_bank(value);
                self.last_chr_bank_set = ChrBankSet::Background;
            }
            0x5130 => self.chr_upper_bits = value & 0x03,
            0x5C00..=0x5FFF => self.write_exram_cpu(address, value),
            _ => {}
        }
    }
}
