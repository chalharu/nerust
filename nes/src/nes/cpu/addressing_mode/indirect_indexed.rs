// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct IndirectIndexed {
    ind_address: usize,
    address_low: u8,
    step: usize,
}

impl IndirectIndexed {
    pub fn new() -> Self {
        Self {
            ind_address: 0,
            address_low: 0,
            step: 0,
        }
    }
}

impl CpuStepState for IndirectIndexed {
    fn entry(
        &mut self,
        _core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) {
        self.step = 0;
    }

    fn exec(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        self.step += 1;
        match self.step {
            1 => {
                self.ind_address = usize::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
            }
            2 => {
                self.address_low = core.memory.read(
                    self.ind_address,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            3 => {
                let address_high = usize::from(core.memory.read(
                    self.ind_address.wrapping_add(1) & 0xFF,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                self.ind_address = (address_high << 8) | usize::from(self.address_low);
                core.register.set_opaddr(
                    self.ind_address
                        .wrapping_add(usize::from(core.register.get_y()))
                        & 0xFFFF,
                );
            }
            4 => {
                if !page_crossed(self.ind_address, core.register.get_opaddr()) {
                    return CpuStepStateEnum::Exit;
                }
                // dummy read
                core.memory.read_dummy_cross(
                    self.ind_address,
                    core.register.get_opaddr(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            _ => {
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }

    fn exit(
        &mut self,
        core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) -> CpuStatesEnum {
        core.opcode_tables.get(core.register.get_opcode())
    }
}
