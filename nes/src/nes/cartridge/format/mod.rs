// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod ines;
use super::error::CartridgeError;
use crate::nes::MirrorMode;
use serde_bytes;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct CartridgeData {
    #[serde(with = "serde_bytes")]
    prog_rom: Vec<u8>,
    #[serde(with = "serde_bytes")]
    char_rom: Vec<u8>,
    #[serde(with = "serde_bytes")]
    sram: Vec<u8>,
    mapper_type: u8,
    mirror_mode: MirrorMode,
    has_battery: bool,
}

impl CartridgeData {
    pub(crate) fn try_from<I: Iterator<Item = u8>>(
        input: &mut I,
    ) -> Result<CartridgeData, CartridgeError> {
        let magic = input.take(4).collect::<Vec<_>>();
        if magic.len() != 4 {
            Err(CartridgeError::UnexpectedEof)
        } else {
            if magic[0] == 0x4e && magic[1] == 0x45 && magic[2] == 0x53 && magic[3] == 0x1a {
                ines::read_ines(input)
            } else {
                Err(CartridgeError::DataError)
            }
        }
    }

    pub(crate) fn mapper_type(&self) -> u8 {
        self.mapper_type
    }

    pub(crate) fn program_bank_offset(&self, mut index: isize) -> usize {
        if index >= 0x80 {
            index -= 0x100;
        }
        index %= (self.prog_rom.len() as isize) / 0x4000;
        let mut offset = index * 0x4000;
        if offset < 0 {
            offset += self.prog_rom.len() as isize;
        }
        offset as usize
    }

    pub(crate) fn program_bank_len(&self) -> usize {
        self.prog_rom.len() / 0x4000
    }

    pub(crate) fn char_bank_offset(&self, mut index: isize) -> usize {
        if index >= 0x80 {
            index -= 0x100;
        }
        index %= (self.char_rom.len() as isize) / 0x1000;
        let mut offset = index * 0x1000;
        if offset < 0 {
            offset += self.char_rom.len() as isize;
        }
        offset as usize
    }

    pub(crate) fn read_prog_rom(&self, index: usize) -> u8 {
        self.prog_rom[index]
    }

    pub(crate) fn read_char_rom(&self, index: usize) -> u8 {
        self.char_rom[index]
    }

    pub(crate) fn read_sram(&self, index: usize) -> u8 {
        self.sram[index]
    }

    // pub(crate) fn write_prog_rom(&mut self, index: usize, data: u8) {
    //     self.prog_rom[index] = data;
    // }

    pub(crate) fn write_char_rom(&mut self, index: usize, data: u8) {
        self.char_rom[index] = data;
    }

    pub(crate) fn write_sram(&mut self, index: usize, data: u8) {
        self.sram[index] = data;
    }

    pub(crate) fn get_mirror_mode(&self) -> MirrorMode {
        self.mirror_mode
    }

    pub(crate) fn set_mirror_mode(&mut self, value: MirrorMode) {
        self.mirror_mode = value;
    }
}
