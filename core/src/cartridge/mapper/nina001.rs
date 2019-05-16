// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Mapper 34

use super::super::{CartridgeDataDao, Mapper, MapperState, MapperStateDao};
use super::{Cartridge, CartridgeData};
use crate::cpu::interrupt::Interrupt;

#[derive(Serialize, Deserialize)]
pub(crate) struct Nina001 {
    cartridge_data: CartridgeData,
    state: MapperState,
}

#[typetag::serde]
impl Cartridge for Nina001 {}

impl Nina001 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
        }
    }
}

impl CartridgeDataDao for Nina001 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }
    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for Nina001 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }
    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for Nina001 {
    fn program_page_len(&self) -> usize {
        0x8000
    }
    fn character_page_len(&self) -> usize {
        0x1000
    }

    fn initialize(&mut self) {
        self.change_program_page(0, 0);
        self.change_character_page(0, 0);
    }

    fn name(&self) -> &str {
        "NINA-001 (Mapper34) "
    }

    fn register_addr(&self, address: usize) -> bool {
        address >= 0x7FFD && address <= 0x7FFF
    }

    fn write_register(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        match address {
            0x7FFD => self.change_program_page(0, usize::from(value & 1)),
            0x7FFE => self.change_character_page(0, usize::from(value & 0x0F)),
            0x7FFF => self.change_character_page(1, usize::from(value & 0x0F)),
            _ => unreachable!(),
        }
    }
}
