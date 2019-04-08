// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Mapper 1

use super::super::{CartridgeDataDao, Mapper, MapperState, MapperStateDao};
use super::{Cartridge, CartridgeData};
use crate::nes::cpu::interrupt::Interrupt;
use crate::nes::MirrorMode;

#[derive(Serialize, Deserialize)]
pub(crate) struct SxRom {
    cartridge_data: CartridgeData,
    state: MapperState,
    control: u8,    // 0x8000 - 0x9FFF
    chr_bank_0: u8, // 0xA000 - 0xBFFF
    chr_bank_1: u8, // 0xC000 - 0xDFFF
    prg_bank: u8,   // 0xE000 - 0xFFFF
    shift_register: u8,
    last_chr_bank: bool, // false: bank0, true: bank1
    cycle: u64,
    prev_cycle: u64,
}

#[typetag::serde]
impl Cartridge for SxRom {}

impl SxRom {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            control: 0x0C,
            prg_bank: 0,
            chr_bank_0: 0,
            chr_bank_1: 0,
            shift_register: 0x10,
            cycle: 0,
            prev_cycle: 0,
            last_chr_bank: false,
        }
    }

    fn write_register_inner(&mut self, address: usize, value: u8) {
        match address {
            0...0x1FFF => self.write_control(value),
            0x2000...0x3FFF => self.write_char_bank_0(value),
            0x4000...0x5FFF => self.write_char_bank_1(value),
            0x6000...0x7FFF => self.write_prog_bank(value),
            _ => {}
        }
    }

    fn write_control(&mut self, value: u8) {
        self.control = value;

        let mirror_mode = match value & 3 {
            0 => MirrorMode::Single0,
            1 => MirrorMode::Single1,
            2 => MirrorMode::Vertical,
            3 => MirrorMode::Horizontal,
            _ => unreachable!(),
        };
        self.set_mirror_mode(mirror_mode);
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
        self.prg_bank = value;
        self.update_offsets();
    }

    fn update_offsets(&mut self) {
        let extra_reg = if self.last_chr_bank && (self.control & 0x10) == 0x10 {
            self.chr_bank_1
        } else {
            self.chr_bank_0
        };

        if (self.prg_bank & 0x10) != 0x10 {
            if self.data_ref().pram_length() + self.data_ref().save_pram_length() > 0x4000 {
                // SXROM ( save 32kb )
                self.change_ram_page(0, usize::from((extra_reg >> 2) & 0x03));
            } else if self.data_ref().pram_length() + self.data_ref().save_pram_length() > 0x2000 {
                if self.data_ref().save_pram_length() == 0x2000
                    && self.data_ref().pram_length() == 0x2000
                {
                    // SOROM ( save 16kb + ram 16kb )
                    self.change_ram_page(
                        0,
                        if (extra_reg >> 3) & 0x01 != 0 {
                            0
                        } else {
                            self.data_ref().save_pram_length() / self.ram_page_len()
                        },
                    );
                } else {
                    // unknown
                    self.change_ram_page(0, usize::from((extra_reg >> 2) & 0x01));
                }
            } else {
                // ram 8kb or nothing
                self.change_ram_page(0, 0);
            }
        }
        if self.data_ref().sub_mapper_type() == 5 {
            self.change_program_page(0, 0);
            self.change_program_page(1, 1);
        } else {
            let prog_bank_sel = if self.data_ref().prog_rom_len() == 0x80000 {
                // 512KB Rom
                extra_reg & 0x10
            } else {
                0
            };
            match (self.control >> 2) & 3 {
                0 | 1 => {
                    // 32k
                    let bank = usize::from((self.prg_bank & 0x0E) | prog_bank_sel);
                    self.change_program_page(0, bank);
                    self.change_program_page(1, bank + 1);
                }
                3 => {
                    // 16k
                    self.change_program_page(
                        0,
                        usize::from((self.prg_bank & 0x0F) | prog_bank_sel),
                    );
                    self.change_program_page(1, usize::from(0x0F | prog_bank_sel));
                }
                _ => {
                    self.change_program_page(0, usize::from(prog_bank_sel));
                    self.change_program_page(
                        1,
                        usize::from((self.prg_bank & 0x0F) | prog_bank_sel),
                    );
                }
            }
        }

        if (self.control & 0x10) == 0x00 {
            // 8k
            self.change_character_page(0, usize::from(self.chr_bank_0 & 0x1E));
            self.change_character_page(1, usize::from((self.chr_bank_0 & 0x1E) + 1));
        } else {
            // 4k
            self.change_character_page(0, usize::from(self.chr_bank_0));
            self.change_character_page(1, usize::from(self.chr_bank_1));
        }
    }
}

impl CartridgeDataDao for SxRom {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }
    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for SxRom {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }
    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for SxRom {
    fn program_page_len(&self) -> usize {
        0x4000
    }
    fn character_page_len(&self) -> usize {
        0x1000
    }

    fn initialize(&mut self) {
        // MMC1A, MMC1BであればWRAMを有効にする必要がある。

        self.write_control(0x0C);
    }

    fn name(&self) -> &str {
        "MMC1 SXROM (Mapper1)"
    }

    fn bus_conflicts(&self) -> bool {
        self.data_ref().sub_mapper_type() == 2
    }

    fn write_register(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        if self.cycle.wrapping_sub(self.prev_cycle) >= 2 {
            if value & 0x80 == 0x80 {
                self.shift_register = 0x10;
                let control = self.control | 0x0C;
                self.write_control(control);
            } else {
                let complete = self.shift_register & 1 == 1;
                let shift_register = (self.shift_register >> 1) | ((value & 1) << 4);
                self.shift_register = if complete {
                    self.write_register_inner(address & 0x7FFF, shift_register);
                    0x10
                } else {
                    shift_register
                };
            }
            self.prev_cycle = self.cycle;
        }
    }

    fn step(&mut self) {
        self.cycle += 1;
    }
}
