// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Mapper 3 or 185

use super::super::{CartridgeDataDao, Mapper, MapperState, MapperStateDao};
use super::{Cartridge, CartridgeData};
use crate::cpu::interrupt::Interrupt;

#[derive(Serialize, Deserialize)]
pub(crate) struct CNRom {
    cartridge_data: CartridgeData,
    state: MapperState,
    protect: bool,
}

#[typetag::serde]
impl Cartridge for CNRom {}

impl CNRom {
    pub(crate) fn new(data: CartridgeData, protect: bool) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            protect,
        }
    }
}

impl CartridgeDataDao for CNRom {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }
    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for CNRom {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }
    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for CNRom {
    fn program_page_len(&self) -> usize {
        0x8000
    }
    fn character_page_len(&self) -> usize {
        0x2000
    }

    fn initialize(&mut self) {
        self.change_program_page(0, 0);
        self.change_character_page(0, 0);
    }

    fn bus_conflicts(&self) -> bool {
        self.protect || self.data_ref().sub_mapper_type() == 2
    }

    fn name(&self) -> &str {
        if self.protect {
            "CNROM (Mapper3) "
        } else {
            "CNROM (Mapper185)"
        }
    }

    fn character_openbus_default(&self) -> Option<u8> {
        Some(0xFF)
    }

    fn write_register(&mut self, _address: usize, value: u8, _interrupt: &mut Interrupt) {
        if self.protect {
            if (self.data_ref().sub_mapper_type() == 16 && (value & 0x01) != 0)
                || (self.data_ref().sub_mapper_type() == 0 && (value & 0x0F) != 0 && value != 0x13)
            {
                self.change_character_page(0, 0);
            } else {
                self.release_character_page(0);
            }
        } else {
            self.change_character_page(0, usize::from(value));
        }
    }
}
