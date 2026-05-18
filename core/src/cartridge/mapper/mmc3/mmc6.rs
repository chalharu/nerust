// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::shared::{Mapper4Config, Mapper4Shared};
use crate::cartridge::format::CartridgeData;
use crate::cartridge::{
    Cartridge, CartridgeDataDao, Mapper, MapperState, MapperStateDao, PpuBusEvent,
};
use crate::cpu::interrupt::Interrupt;

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(super) struct Mmc6 {
    pub(super) shared: Mapper4Shared,
}

impl Mmc6 {
    pub(super) fn new(data: CartridgeData) -> Self {
        Self {
            shared: Mapper4Shared::new(data, Mapper4Config::mmc6()),
        }
    }
}

#[typetag::serde]
impl Cartridge for Mmc6 {}

impl CartridgeDataDao for Mmc6 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        self.shared.data_mut()
    }

    fn data_ref(&self) -> &CartridgeData {
        self.shared.data_ref()
    }
}

impl MapperStateDao for Mmc6 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        self.shared.mapper_state_mut()
    }

    fn mapper_state_ref(&self) -> &MapperState {
        self.shared.mapper_state_ref()
    }
}

impl Mapper for Mmc6 {
    fn name(&self) -> &str {
        "MMC6 (Mapper4)"
    }

    fn program_page_len(&self) -> usize {
        self.shared.program_page_len()
    }

    fn character_page_len(&self) -> usize {
        self.shared.character_page_len()
    }

    fn read_ram(&self, index: usize) -> Option<u8> {
        self.shared.read_ram(index)
    }

    fn write_ram(&mut self, index: usize, data: u8) {
        self.shared.write_ram(index, data);
    }

    fn save_len_default(&self) -> usize {
        self.shared.save_len_default()
    }

    fn ram_len_default(&self) -> usize {
        self.shared.ram_len_default()
    }

    fn ram_page_len_default(&self) -> usize {
        self.shared.ram_page_len_default()
    }

    fn battery_default(&self) -> bool {
        self.shared.battery_default()
    }

    fn initialize(&mut self) {
        self.shared.initialize();
    }

    fn bus_conflicts(&self) -> bool {
        self.shared.bus_conflicts()
    }

    fn write_register(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        self.shared.write_register(address, value, interrupt);
    }

    fn notify_ppu_bus_event(&mut self, event: PpuBusEvent, interrupt: &mut Interrupt) {
        self.shared.notify_ppu_bus_event(event, interrupt);
    }
}
