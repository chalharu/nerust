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
pub(crate) struct Mapper1 {
    shift_register: u8,
    control: u8,
    prg_mode: u8,
    chr_mode: u8,
    prg_bank: u8,
    chr_bank_0: u8,
    chr_bank_1: u8,
    prg_offsets: [usize; 2],
    chr_offsets: [usize; 2],
    cartridge_data: CartridgeData,
}

impl Mapper1 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            prg_offsets: [0, data.program_bank_offset(-1)],
            cartridge_data: data,
            shift_register: 0x10,
            control: 0,
            prg_mode: 0,
            chr_mode: 0,
            prg_bank: 0,
            chr_bank_0: 0,
            chr_bank_1: 0,
            chr_offsets: [0; 2],
        }
    }

    fn load_register(&mut self, address: usize, value: u8) {
        if value & 0x80 == 0x80 {
            self.shift_register = 0x10;
            let control = self.control | 0x0C;
            self.write_control(control);
        } else {
            let complete = self.shift_register & 1 == 1;
            let shift_register = (self.shift_register >> 1) | ((value & 1) << 4);
            self.shift_register = if complete {
                self.write_register(address, shift_register);
                0x10
            } else {
                shift_register
            };
        }
    }

    fn write_register(&mut self, address: usize, value: u8) {
        match address {
            0...0x9FFF => self.write_control(value),
            0xA000...0xBFFF => self.write_char_bank_0(value),
            0xC000...0xdFFF => self.write_char_bank_1(value),
            0xE000...0xFFFF => self.write_prog_bank(value),
            _ => {}
        }
    }

    fn write_control(&mut self, value: u8) {
        self.control = value;
        self.chr_mode = (value >> 4) & 1;
        self.prg_mode = (value >> 2) & 3;
        let mirror_mode = match value & 3 {
            0 => MirrorMode::Single0,
            1 => MirrorMode::Single1,
            2 => MirrorMode::Vertical,
            3 => MirrorMode::Horizontal,
            _ => self.cartridge_data.get_mirror_mode(),
        };
        self.cartridge_data.set_mirror_mode(mirror_mode);
        self.update_offsets();
    }

    fn write_char_bank_0(&mut self, value: u8) {
        self.chr_bank_0 = value;
        self.update_offsets();
    }

    fn write_char_bank_1(&mut self, value: u8) {
        self.chr_bank_1 = value;
        self.update_offsets();
    }

    fn write_prog_bank(&mut self, value: u8) {
        self.prg_bank = value & 0x0F;
        self.update_offsets();
    }

    fn update_offsets(&mut self) {
        match self.prg_mode {
            0 | 1 => {
                self.prg_offsets[0] = self
                    .cartridge_data
                    .program_bank_offset((self.prg_bank & 0xfe) as isize);
                self.prg_offsets[1] = self
                    .cartridge_data
                    .program_bank_offset((self.prg_bank | 0x01) as isize);
            }
            2 => {
                self.prg_offsets[0] = 0;
                self.prg_offsets[1] = self
                    .cartridge_data
                    .program_bank_offset(self.prg_bank as isize);
            }
            3 => {
                self.prg_offsets[0] = self
                    .cartridge_data
                    .program_bank_offset(self.prg_bank as isize);
                self.prg_offsets[1] = self.cartridge_data.program_bank_offset(-1);
            }
            _ => {}
        }
        match self.chr_mode {
            0 => {
                self.chr_offsets[0] = self
                    .cartridge_data
                    .char_bank_offset((self.chr_bank_0 & 0xfe) as isize);
                self.chr_offsets[1] = self
                    .cartridge_data
                    .char_bank_offset((self.chr_bank_0 | 0x01) as isize);
            }
            1 => {
                self.chr_offsets[0] = self
                    .cartridge_data
                    .char_bank_offset(self.chr_bank_0 as isize);
                self.chr_offsets[1] = self
                    .cartridge_data
                    .char_bank_offset(self.chr_bank_1 as isize);
            }
            _ => {}
        }
    }
}

impl Cartridge for Mapper1 {
    fn read(&self, mut address: usize) -> OpenBusReadResult {
        OpenBusReadResult::new(
            match address {
                0...0x1FFF => {
                    let bank = address / 0x1000;
                    let offset = address & 0xFFF;
                    self.cartridge_data
                        .read_char_rom(self.chr_offsets[bank] + offset)
                }
                0x6000...0x7FFF => self.cartridge_data.read_sram(address - 0x6000),
                n if n >= 0x8000 => {
                    address -= 0x8000;
                    let bank = address / 0x4000;
                    let offset = address & 0x3FFF;
                    self.cartridge_data
                        .read_prog_rom(self.prg_offsets[bank] + offset)
                }
                _ => {
                    error!("unhandled mapper1 read at address: 0x{:04X}", address);
                    0
                }
            },
            0xFF,
        )
    }

    fn write(&mut self, address: usize, value: u8) {
        match address {
            0...0x1FFF => {
                let bank = address / 0x1000;
                let offset = address & 0xFFF;
                self.cartridge_data
                    .write_char_rom(self.chr_offsets[bank] + offset, value);
            }
            0x6000...0x7FFF => self.cartridge_data.write_sram(address - 0x6000, value),
            n if n >= 0x8000 => {
                self.load_register(address, value);
            }
            _ => {
                error!("unhandled mapper1 write at address: 0x{:04X}", address);
            }
        }
    }

    fn name(&self) -> &str {
        "Mapper1(MMC1)"
    }

    fn step(&mut self) {}

    fn mirror_mode(&self) -> MirrorMode {
        self.cartridge_data.get_mirror_mode()
    }
}
