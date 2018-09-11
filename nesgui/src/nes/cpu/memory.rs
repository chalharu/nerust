// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nes::{Apu, Cartridge, Controller, Ppu};

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct MemoryState {
    lastread: u8,
}

impl MemoryState {
    pub fn new() -> Self {
        Self { lastread: 0 }
    }
}

pub(crate) struct Memory<'a> {
    wram: &'a mut [u8; 2048],
    ppu: &'a mut Ppu,
    apu: &'a mut Apu,
    controller: &'a mut Controller,
    cartridge: &'a mut Box<Cartridge>,
    state: &'a mut MemoryState,
}

impl<'a> Memory<'a> {
    pub fn new<C: Controller>(
        wram: &'a mut [u8; 2048],
        ppu: &'a mut Ppu,
        apu: &'a mut Apu,
        controller: &'a mut C,
        cartridge: &'a mut Box<Cartridge>,
        state: &'a mut MemoryState,
    ) -> Self {
        Self {
            wram,
            ppu,
            apu,
            controller,
            cartridge,
            state,
        }
    }

    pub fn read(&mut self, address: usize) -> u8 {
        let result = match address {
            0...0x1FFF => self.wram[address & 0x07FF],
            0x2000...0x3FFF => self
                .ppu
                .read_register(0x2000 + (address & 7), self.cartridge),
            // 0x4014 => self.ppu.read_register(address, self.cartridge),
            0x4015 => self.apu.read_register(address),
            0x4016 | 0x4017 => {
                (self.controller.read(address & 1) & 0x1F) | (self.state.lastread & 0xE0)
            }
            0x4000...0x5FFF => self.state.lastread, // TODO: I/O registers
            0x6000...0x10000 => self.cartridge.read(address),
            _ => {
                error!("unhandled cpu memory read at address: 0x{:04X}", address);
                self.state.lastread
            }
        };
        self.state.lastread = result;
        result
    }

    pub fn read_u16(&mut self, address: usize) -> u16 {
        let low = u16::from(self.read(address));
        let high = u16::from(self.read(address + 1));
        (high << 8) | low
    }

    pub fn read_u16_bug(&mut self, address: usize) -> u16 {
        let low = u16::from(self.read(address));
        let high = u16::from(self.read((address & 0xFF00) | ((address + 1) & 0xFF)));
        (high << 8) | low
    }

    pub fn write(&mut self, address: usize, value: u8) -> usize {
        match address {
            0...0x1FFF => {
                self.wram[address & 0x07FF] = value;
                0
            }
            0x2000...0x3FFF => {
                self.ppu
                    .write_register(0x2000 + (address & 7), value, self.cartridge);
                0
            }
            0x4000...0x4013 => {
                self.apu.write_register(address, value);
                0
            }
            0x4014 => {
                let v = (0..256)
                    .map(|i| self.read((usize::from(value) << 8) | i))
                    .collect::<Vec<_>>();
                self.ppu.write_dma(&v);
                // TODO: もしCPUサイクルが奇数だったら、本当はもう１サイクル追加する必要がある。
                513
            }
            0x4015 => {
                self.apu.write_register(address, value);
                0
            }
            0x4016 => {
                self.controller.write(value);
                0
            }
            0x4017 => {
                self.apu.write_register(address, value);
                0
            }
            0x4018...0x5FFF => 0, // TODO: I/O registers
            0x6000...0xFFFF => {
                self.cartridge.write(address, value);
                0
            }
            _ => {
                error!("unhandled cpu memory write at address: 0x{:04X}", address);
                0
            }
        }
    }
}
