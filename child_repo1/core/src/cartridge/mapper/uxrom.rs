// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Mapper 2

use super::super::{CartridgeDataDao, Mapper, MapperState, MapperStateDao};
use super::{Cartridge, CartridgeData};
use crate::cpu::interrupt::Interrupt;

#[derive(Serialize, Deserialize)]
pub(crate) struct UxRom {
    cartridge_data: CartridgeData,
    state: MapperState,
}

#[typetag::serde]
impl Cartridge for UxRom {}

impl UxRom {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
        }
    }
}

impl CartridgeDataDao for UxRom {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }
    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for UxRom {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }
    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for UxRom {
    fn program_page_len(&self) -> usize {
        0x4000
    }
    fn character_page_len(&self) -> usize {
        0x2000
    }

    fn initialize(&mut self) {
        self.change_program_page(0, 0);
        let lastpage = (self.data_ref().prog_rom_len() / self.program_page_len()) - 1;
        self.change_program_page(1, lastpage);
        self.change_character_page(0, 0);
    }

    fn name(&self) -> &str {
        "UXROM (Mapper2)"
    }

    fn bus_conflicts(&self) -> bool {
        self.data_ref().sub_mapper_type() == 2
    }

    fn write_register(&mut self, _address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.change_program_page(0, usize::from(value));
    }
}
