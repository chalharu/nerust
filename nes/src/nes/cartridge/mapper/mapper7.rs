// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::super::Cartridge;
use super::CartridgeData;
use crate::nes::MirrorMode;
use std::cmp;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Mapper7 {
    prg_banks: u8,
    prg_bank1: u8,
    prg_bank2: u8,
    cartridge_data: CartridgeData,
    mirror_mode: MirrorMode,
}

impl Mapper7 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        let prg_banks = data.program_bank_len() as u8;
        Self {
            prg_bank1: 0,
            prg_bank2: cmp::min(1, prg_banks - 1),
            prg_banks,
            mirror_mode: data.get_mirror_mode(),
            cartridge_data: data,
        }
    }

    fn set_register(&mut self, value: u8) {
        self.mirror_mode = if (value & 0x10) == 0 {
            MirrorMode::Single0
        } else {
            MirrorMode::Single1
        };
        self.prg_bank1 = cmp::min(value & 0x07, (self.prg_banks - 1) >> 1) << 1;
        self.prg_bank2 = cmp::min(self.prg_bank1 + 1, self.prg_banks - 1);
    }
}

impl Cartridge for Mapper7 {
    fn read(&self, address: usize) -> u8 {
        match address {
            0...0x1FFF => self.cartridge_data.read_char_rom(address),
            0x6000...0x7FFF => self.cartridge_data.read_sram(address - 0x6000),
            0x8000...0xBFFF => self
                .cartridge_data
                .read_prog_rom(usize::from(self.prg_bank1) * 0x4000 + address - 0x8000),
            n if n >= 0xC000 => self
                .cartridge_data
                .read_prog_rom(usize::from(self.prg_bank2) * 0x4000 + address - 0xC000),
            _ => {
                error!("unhandled mapper7 read at address: 0x{:04X}", address);
                0
            }
        }
    }

    fn write(&mut self, address: usize, value: u8) {
        match address {
            0...0x1FFF => {
                self.cartridge_data.write_char_rom(address, value);
            }
            0x6000...0x7FFF => self.cartridge_data.write_sram(address - 0x6000, value),
            n if n >= 0x8000 => self.set_register(value),
            _ => {
                error!("unhandled mapper7 write at address: 0x{:04X}", address);
            }
        }
    }

    fn step(&mut self) {}

    fn name(&self) -> &str {
        "Mapper7(AxROM)"
    }

    fn mirror_mode(&self) -> MirrorMode {
        self.mirror_mode
    }
}
