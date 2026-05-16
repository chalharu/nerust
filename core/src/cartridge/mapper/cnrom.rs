// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Mapper 3 or 185

use super::super::{CartridgeDataDao, Mapper, MapperState, MapperStateDao};
use super::{Cartridge, CartridgeData};
use crate::cpu::interrupt::Interrupt;

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct CNRom {
    cartridge_data: CartridgeData,
    state: MapperState,
    protect: bool,
}

#[typetag::serde]
impl Cartridge for CNRom {}

impl CNRom {
    pub(crate) fn new_mapper3(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            protect: false,
        }
    }

    pub(crate) fn new_mapper185(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            protect: true,
        }
    }

    fn is_mapper185(&self) -> bool {
        self.protect
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
        self.is_mapper185() || self.data_ref().sub_mapper_type() == 2
    }

    fn name(&self) -> &str {
        if self.is_mapper185() {
            "CNROM (Mapper185)"
        } else {
            "CNROM (Mapper3)"
        }
    }

    fn character_openbus_default(&self) -> Option<u8> {
        Some(0xFF)
    }

    fn write_register(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        if self.is_mapper185() {
            if (self.data_ref().sub_mapper_type() == 16 && (value & 0x01) != 0)
                || (self.data_ref().sub_mapper_type() == 0 && (value & 0x0F) != 0 && value != 0x13)
            {
                self.change_character_page(0, 0);
            } else {
                self.release_character_page(0);
            }
        } else {
            if self.data_ref().sub_mapper_type() == 0
                && self.read_program(address - 0x8000).data != value
            {
                // Mapper 3 submapper 0 is unspecified. Preserve the current bank
                // unless the ROM write is conflict-safe, so we do not silently
                // collapse unknown hardware into either submapper 1 or 2.
                return;
            }
            self.change_character_page(0, usize::from(value));
        }
    }
}
