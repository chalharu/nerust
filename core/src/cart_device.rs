// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::cpu::interrupt::Interrupt;
use crate::mapper::Mapper;
use crate::mapper_state::MappingMode;
use crate::{MirrorMode, OpenBusReadResult};
use std::cmp;

#[typetag::serde(tag = "type")]
pub(crate) trait Cartridge: Mapper {
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
        self.set_mirror_mode(self.data_ref().mirror_mode());
        Mapper::initialize(self);
    }

    fn read(&self, address: usize) -> OpenBusReadResult {
        match address {
            0..=0x1FFF => self.read_character(address),
            0x4020..=0x5FFF => Mapper::read_expansion(self, address),
            0x6000..=0x7FFF => Cartridge::read_ram(self, address - 0x6000),
            0x8000..=0xFFFF => self.read_program(address - 0x8000),
            _ => {
                log::error!("unhandled mapper read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0)
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
        Mapper::read_ram(self, address).map_or_else(
            || OpenBusReadResult::new(0, 0),
            |x| OpenBusReadResult::new(x, 0xFF),
        )
    }

    fn read_program(&self, address: usize) -> OpenBusReadResult {
        OpenBusReadResult::new(
            self.program_address(address)
                .map(|x| self.data_ref().read_prog_rom(x))
                .unwrap_or(0),
            0xFF,
        )
    }

    fn write(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        match address {
            0..=0x1FFF => self.write_character(address, value),
            0x4020..=0x5FFF => Mapper::write_expansion(self, address, value, interrupt),
            0x6000..=0x7FFF => Cartridge::write_ram(self, address, value, interrupt),
            0x8000..=0xFFFF => self.write_program(address, value, interrupt),
            _ => {
                log::error!("unhandled mapper write at address: 0x{:04X}", address);
            }
        }
    }

    fn write_character(&mut self, address: usize, value: u8) {
        if self.mapper_state_ref().character_mapping_mode == MappingMode::Ram
            && let Some(addr) = self.character_address(address)
        {
            self.mapper_state_mut().vram[addr] = value;
        }
    }

    fn write_ram(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        if self.register_addr(address) {
            self.write_register(
                address,
                if self.bus_conflicts() {
                    Mapper::read_ram(self, address - 0x6000).unwrap_or(0) & value
                } else {
                    value
                },
                interrupt,
            );
        } else {
            Mapper::write_ram(self, address - 0x6000, value);
        }
    }

    fn write_program(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        if self.register_addr(address) {
            self.write_register(
                address,
                if self.bus_conflicts() {
                    self.read_program(address - 0x8000).data & value
                } else {
                    value
                },
                interrupt,
            );
        }
    }

    fn mirror_mode(&self) -> MirrorMode {
        self.get_mirror_mode()
    }
}

// 本当はこうしたい
// #[typetag::serde]
// impl<T: Mapper> Cartridge for T {}
