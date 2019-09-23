// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::cpu::{Interrupt, Register};
use crate::{Apu, Cartridge, Controller, Ppu};
use crate::{OpenBus, OpenBusReadResult};

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Memory {
    #[serde(with = "nerust_serialize::BigArray")]
    wram: [u8; 2048],
    openbus: OpenBus,
}

impl Memory {
    pub(crate) fn new() -> Self {
        Self {
            wram: [0; 2048],
            openbus: OpenBus::new(),
        }
    }

    pub(crate) fn read_next(
        &mut self,
        register: &mut Register,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
        interrupt: &mut Interrupt,
    ) -> u8 {
        let pc = register.get_pc();
        register.set_pc(pc.wrapping_add(1));
        self.read(pc as usize, ppu, cartridge, controller, apu, interrupt)
    }

    pub(crate) fn read(
        &mut self,
        address: usize,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
        interrupt: &mut Interrupt,
    ) -> u8 {
        let result = match address {
            0..=0x1FFF => OpenBusReadResult::new(self.wram[address & 0x07FF], 0xFF),
            0x2000..=0x3FFF => ppu.read_register(0x2000 + (address & 7), cartridge, interrupt),
            0x4015 => apu.read_register(address, interrupt),
            0x4016 | 0x4017 => controller.read(address & 1),
            0x4000..=0x4014 | 0x4018..=0x5FFF => OpenBusReadResult::new(0, 0), // TODO: I/O registers
            0x6000..=0xFFFF => cartridge.read(address),
            _ => {
                log::error!("unhandled cpu memory read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0)
            }
        };
        interrupt.write = false;
        self.openbus.unite(result)
    }

    pub(crate) fn read_dummy_cross(
        &mut self,
        address: usize,
        new_address: usize,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
        interrupt: &mut Interrupt,
    ) {
        let _ = self.read(
            (address & 0xFF00) | (new_address & 0xFF),
            ppu,
            cartridge,
            controller,
            apu,
            interrupt,
        );
    }

    pub(crate) fn write(
        &mut self,
        address: usize,
        value: u8,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
        interrupt: &mut Interrupt,
    ) {
        match address {
            0..=0x1FFF => self.wram[address & 0x07FF] = value,
            0x2000..=0x3FFF => {
                ppu.write_register(0x2000 + (address & 7), value, cartridge, interrupt)
            }
            0x4000..=0x4013 => apu.write_register(address, value, interrupt),
            0x4014 => interrupt.oam_dma = Some(value),
            0x4015 => apu.write_register(address, value, interrupt),
            0x4016 => controller.write(value),
            0x4017 => apu.write_register(address, value, interrupt),
            0x4018..=0x5FFF => (), // TODO: I/O registers
            0x6000..=0xFFFF => cartridge.write(address, value, interrupt),
            _ => {
                log::error!("unhandled cpu memory write at address: 0x{:04X}", address);
            }
        }
        interrupt.write = true;
    }
}
