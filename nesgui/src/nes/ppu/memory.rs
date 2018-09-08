// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;
use nes::Cartridge;

pub(crate) struct Memory<'a> {
    state: &'a mut State,
    cartridge: &'a mut Box<Cartridge>,
}

impl<'a> Memory<'a> {
    pub fn new(state: &'a mut State, cartridge: &'a mut Box<Cartridge>) -> Self {
        Self { state, cartridge }
    }

    pub fn read(&self, mut address: usize) -> u8 {
        address &= 0x3FFF;
        match address {
            0...0x1FFF => self.cartridge.read(address),
            2000...0x3EFF => {
                self.state.vram[self.cartridge.mirror_mode().mirror_address(address) & 0x7FF]
            }
            0x3F00...0x3FFF => self.state.read_palette(address),
            _ => {
                error!("unhandled ppu memory read at address: 0x{:04X}", address);
                0
            }
        }
    }

    pub fn write(&mut self, mut address: usize, value: u8) {
        address &= 0x3FFF;
        match address {
            0...0x1FFF => self.cartridge.write(address, value),
            2000...0x3EFF => {
                self.state.vram[self.cartridge.mirror_mode().mirror_address(address) & 0x7FF] =
                    value
            }
            0x3F00...0x3FFF => self.state.write_palette(address, value),
            _ => error!("unhandled ppu memory write at address: 0x{:04X}", address),
        }
    }
}
