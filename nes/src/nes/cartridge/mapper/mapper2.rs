// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::super::Cartridge;
use super::CartridgeData;
use crate::nes::MirrorMode;
use crate::nes::OpenBusReadResult;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Mapper2 {
    prg_banks: u8,
    prg_bank1: u8,
    prg_bank2: u8,
    cartridge_data: CartridgeData,
}

impl Mapper2 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        let prg_banks = data.program_bank_len() as u8;
        Self {
            prg_banks,
            prg_bank1: 0,
            prg_bank2: prg_banks - 1,
            cartridge_data: data,
        }
    }
}

impl Cartridge for Mapper2 {
    fn read(&self, address: usize) -> OpenBusReadResult {
        OpenBusReadResult::new(
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
                    error!("unhandled mapper2 read at address: 0x{:04X}", address);
                    0
                }
            },
            0xFF,
        )
    }

    fn write(&mut self, address: usize, value: u8) {
        match address {
            0...0x1FFF => {
                self.cartridge_data.write_char_rom(address, value);
            }
            0x6000...0x7FFF => self.cartridge_data.write_sram(address - 0x6000, value),
            n if n >= 0x8000 => {
                self.prg_bank1 = value % self.prg_banks;
            }
            _ => {
                error!("unhandled mapper2 write at address: 0x{:04X}", address);
            }
        }
    }

    fn step(&mut self) {}

    fn name(&self) -> &str {
        "Mapper2(UxROM)"
    }

    fn mirror_mode(&self) -> MirrorMode {
        self.cartridge_data.get_mirror_mode()
    }
}
