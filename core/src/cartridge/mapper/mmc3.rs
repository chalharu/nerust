// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Mapper 4

use super::super::{CartridgeDataDao, Mapper, MapperState, MapperStateDao};
use super::{CartridgeData, Cartridge};
use crate::MirrorMode;
use crate::cpu::interrupt::{Interrupt, IrqSource};

#[derive(Serialize, Deserialize)]
pub(crate) struct Mmc3 {
    cartridge_data: CartridgeData,
    state: MapperState,
    bank_select: u8, // $8000-$9FFE, even
    bank_data: [u8; 8], // $8000-$9FFE, odd
    mirroring: u8, // $A000-$BFFE, even
    program_ram_protect: u8, // $A001-$BFFF, odd
    irq_latch: u8, // $C000-$DFFE, even
    irq_reload: bool, // $C001-$DFFF, odd
    irq_counter: u8,
    irq_enabled: bool, // disable = $E000-$FFFE, even , enable = $E001-$FFFF, odd
    irq_next: bool,
    cycle: u64,
    prev_cycle: u64,
}

#[typetag::serde]
impl Cartridge for Mmc3 {}

impl Mmc3 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            bank_select: 0,
            bank_data: [0; 8],
            mirroring: 0,
            program_ram_protect: 0,
            irq_latch: 0,
            irq_reload: false,
            irq_counter: 0,
            irq_enabled: false,
            irq_next: false,
            cycle: 0,
            prev_cycle: 0,
        }
    }

    fn write_bank_select(&mut self, value: u8) {
        self.bank_select = value;
        self.update_offsets();
    }

    fn write_bank_data(&mut self, value: u8) {
        let selecter = self.bank_select & 0x07;
        self.bank_data[selecter] = if selecter <= 1 {
            value & !0x01
        } else { value};
        self.update_offsets();
    }

    fn write_mirroring(&mut self, value: u8) {
        self.mirroring = value;

        if self.get_mirror_mode() != MirrorMode::Four {
            self.set_mirror_mode(match value & 1 {
                0 => MirrorMode::Vertical,
                1 => MirrorMode::Horizontal,
                _ => unreachable!(),
            });
        }
    }

    fn write_irq_latch(&mut self, value: u8) {
        self.irq_latch = value;
    }

    fn write_irq_reload(&mut self, _value: u8) {
        self.irq_counter = 0;
        self.irq_reload = true;
    }

    fn write_disable_irq(&mut self, _value: u8, interrupt: &mut Interrupt) {
        self.irq_enabled = false;
        interrupt.clear_irq(IrqSource::External);
    }

    fn write_enable_irq(&mut self, _value: u8) {
        self.irq_enabled = true;
    }

    fn update_offsets(&mut self) {
/*
			if(_romInfo.MapperID == 4 && _romInfo.SubMapperID == 1) {
				//bool wramEnabled = (_state.Reg8000 & 0x20) == 0x20;
				RemoveCpuMemoryMapping(0x6000, 0x7000);

				uint8_t firstBankAccess = (_state.RegA001 & 0x10 ? MemoryAccessType::Write : 0) | (_state.RegA001 & 0x20 ? MemoryAccessType::Read : 0);
				uint8_t lastBankAccess = (_state.RegA001 & 0x40 ? MemoryAccessType::Write : 0) | (_state.RegA001 & 0x80 ? MemoryAccessType::Read : 0);

				for(int i = 0; i < 4; i++) {
					SetCpuMemoryMapping(0x7000 + i * 0x400, 0x71FF + i * 0x400, 0, PrgMemoryType::SaveRam, firstBankAccess);
					SetCpuMemoryMapping(0x7200 + i * 0x400, 0x73FF + i * 0x400, 1, PrgMemoryType::SaveRam, lastBankAccess);
				}
			} else {
				_wramEnabled = (_state.RegA001 & 0x80) == 0x80;
				_wramWriteProtected = (_state.RegA001 & 0x40) == 0x40;

				if(_romInfo.SubMapperID == 0) {
					if(_wramEnabled) {
						SetCpuMemoryMapping(0x6000, 0x7FFF, 0, HasBattery() ? PrgMemoryType::SaveRam : PrgMemoryType::WorkRam, _wramEnabled && !_wramWriteProtected ? MemoryAccessType::ReadWrite : MemoryAccessType::Read);
					} else {
						RemoveCpuMemoryMapping(0x6000, 0x7FFF);
					}
				}
			}

			_chrMode = (_state.Reg8000 & 0x80) >> 7;
			_prgMode = (_state.Reg8000 & 0x40) >> 6;

			if(_chrMode == 0) {
				SelectCHRPage(0, _registers[0] & 0xFE);
				SelectCHRPage(1, _registers[0] | 0x01);
				SelectCHRPage(2, _registers[1] & 0xFE);
				SelectCHRPage(3, _registers[1] | 0x01);

				SelectCHRPage(4, _registers[2]);
				SelectCHRPage(5, _registers[3]);
				SelectCHRPage(6, _registers[4]);
				SelectCHRPage(7, _registers[5]);
			} else if(_chrMode == 1) {
				SelectCHRPage(0, _registers[2]);
				SelectCHRPage(1, _registers[3]);
				SelectCHRPage(2, _registers[4]);
				SelectCHRPage(3, _registers[5]);

				SelectCHRPage(4, _registers[0] & 0xFE);
				SelectCHRPage(5, _registers[0] | 0x01);
				SelectCHRPage(6, _registers[1] & 0xFE);
				SelectCHRPage(7, _registers[1] | 0x01);
			}
			if(_prgMode == 0) {
				SelectPRGPage(0, _registers[6]);
				SelectPRGPage(1, _registers[7]);
				SelectPRGPage(2, -2);
				SelectPRGPage(3, -1);
			} else if(_prgMode == 1) {
				SelectPRGPage(0, -2);
				SelectPRGPage(1, _registers[7]);
				SelectPRGPage(2, _registers[6]);
				SelectPRGPage(3, -1);
			}
*/

        }
}

impl CartridgeDataDao for Mmc3 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }
    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for Mmc3 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }
    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for Mmc3 {
    fn program_page_len(&self) -> usize {
        0x2000
    }

    fn character_page_len(&self) -> usize {
        0x0400
    }

    fn save_len_default(&self) -> usize {
        if self.data_ref().sub_mapper_type() == 1 {
            0x0400
        } else {
            0x2000
        }
    }

    fn ram_len_default(&self) -> usize {
        if self.data_ref().sub_mapper_type() == 1 {
            0x0400
        } else {
            0x2000
        }
    }

    fn ram_page_len_default(&self) -> usize {
        if self.data_ref().sub_mapper_type() == 1 {
            0x0200
        } else {
            0x2000
        }
    }

    fn battery_default(&self) -> bool {
        true
    }
    fn initialize(&mut self) {
        self.write_control(0x0C);
    }

    fn name(&self) -> &str {
        "MMC3 (Mapper4)"
    }

    fn bus_conflicts(&self) -> bool {
        self.data_ref().sub_mapper_type() == 2
    }

    fn write_register(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        match address & 0x6001 {
            0x0000 => self.write_bank_select(value),
            0x0001 => self.write_bank_data(value),
            0x2000 => self.write_mirroring(value),
            0x2001 => self.write_program_ram_protect(value),
            0x4000 => self.write_irq_latch(value),
            0x4001 => self.write_irq_reload(value),
            0x6000 => self.write_disable_irq(value, interrupt),
            0x6001 => self.write_enable_irq(value),
            _ => {}
        }
    }

    fn step(&mut self) {
        self.cycle += 1;
    }

    fn vram_address_change(&mut self, address: usize) {

    }
}
