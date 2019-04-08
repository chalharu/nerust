// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Mapper 7

use super::super::{CartridgeDataDao, Mapper, MapperState, MapperStateDao};
use super::{Cartridge, CartridgeData};
use crate::nes::cpu::interrupt::Interrupt;
use crate::nes::MirrorMode;

#[derive(Serialize, Deserialize)]
pub(crate) struct AxRom {
    cartridge_data: CartridgeData,
    state: MapperState,
}

#[typetag::serde]
impl Cartridge for AxRom {}

impl AxRom {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
        }
    }
}

impl CartridgeDataDao for AxRom {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }
    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for AxRom {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }
    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for AxRom {
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

    fn name(&self) -> &str {
        "AXROM (Mapper7)"
    }

    fn bus_conflicts(&self) -> bool {
        self.data_ref().sub_mapper_type() == 2
    }

    fn write_register(&mut self, _address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.change_program_page(0, usize::from(value) & 0x0F);
        if value & 0x10 == 0x10 {
            self.set_mirror_mode(MirrorMode::Single1);
        } else {
            self.set_mirror_mode(MirrorMode::Single0);
        }
    }
}
