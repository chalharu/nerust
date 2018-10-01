// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::nes::cpu::{Register, Interrupt};
use crate::nes::{Apu, Cartridge, Controller, Ppu};

pub(crate) struct Memory {
    wram: [u8; 2048],
    lastread: u8,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            wram: [0; 2048],
            lastread: 0,
        }
    }

    pub fn read_next(
        &mut self,
        register: &mut Register,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
        interrupt: &mut Interrupt,
    ) -> u8 {
        let pc = register.get_pc();
        register.set_pc(pc.wrapping_add(1));
        self.read(pc as usize, ppu, cartridge, controller, apu, interrupt)
    }

    pub fn read(
        &mut self,
        address: usize,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
        interrupt: &mut Interrupt,
    ) -> u8 {
        let result = match address {
            0...0x1FFF => self.wram[address & 0x07FF],
            0x2000...0x3FFF => ppu.read_register(0x2000 + (address & 7), cartridge, interrupt),
            0x4015 => apu.read_register(address),
            0x4016 | 0x4017 => (controller.read(address & 1) & 0x1F) | (self.lastread & 0xE0),
            0x4000...0x5FFF => self.lastread, // TODO: I/O registers
            0x6000...0x10000 => cartridge.read(address),
            _ => {
                error!("unhandled cpu memory read at address: 0x{:04X}", address);
                self.lastread
            }
        };
        self.lastread = result;
        result
    }

    pub fn read_dummy(
        &mut self,
        address: usize,
        new_address: usize,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
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

    // pub fn read_u16<C: Controller>(
    //     &mut self,
    //     address: usize,
    //     ppu: &mut Ppu,
    //     cartridge: &mut Box<Cartridge>,
    //     controller: &mut C,
    //     apu: &mut Apu,
    // ) -> u16 {
    //     let low = u16::from(self.read(address, ppu, cartridge, controller, apu));
    //     let high = u16::from(self.read(address + 1, ppu, cartridge, controller, apu));
    //     (high << 8) | low
    // }

    // pub fn read_u16_bug<C: Controller>(
    //     &mut self,
    //     address: usize,
    //     ppu: &mut Ppu,
    //     cartridge: &mut Box<Cartridge>,
    //     controller: &mut C,
    //     apu: &mut Apu,
    // ) -> u16 {
    //     let low = u16::from(self.read(address));
    //     let high = u16::from(self.read((address & 0xFF00) | ((address + 1) & 0xFF)));
    //     (high << 8) | low
    // }

    pub fn write(
        &mut self,
        address: usize,
        value: u8,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
        interrupt: &mut Interrupt,
    ) {
        match address {
            0...0x1FFF => self.wram[address & 0x07FF] = value,
            0x2000...0x3FFF => {
                ppu.write_register(0x2000 + (address & 7), value, cartridge, interrupt)
            }
            0x4000...0x4013 => apu.write_register(address, value),
            0x4014 => {
                // let v = (0..256)
                //     .map(|i| self.read((usize::from(value) << 8) | i))
                //     .collect::<Vec<_>>();
                // ppu.write_dma(&v);
            }
            0x4015 => apu.write_register(address, value),

            0x4016 => controller.write(value),

            0x4017 => apu.write_register(address, value),
            0x4018...0x5FFF => (), // TODO: I/O registers
            0x6000...0xFFFF => cartridge.write(address, value),
            _ => {
                error!("unhandled cpu memory write at address: 0x{:04X}", address);
            }
        }
    }
}
