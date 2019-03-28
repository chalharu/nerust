// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct AbsoluteY {
    temp_address: usize,
    step: usize,
}

impl AbsoluteY {
    pub fn new() -> Self {
        Self {
            temp_address: 0,
            step: 0,
        }
    }
}

impl CpuStepState for AbsoluteY {
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
                self.temp_address = usize::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
            }
            2 => {
                let address_high = core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
                self.temp_address |= usize::from(address_high) << 8;
                core.register.set_opaddr(
                    self.temp_address
                        .wrapping_add(usize::from(core.register.get_y()))
                        & 0xFFFF,
                );
            }
            3 => {
                if !page_crossed(self.temp_address, core.register.get_opaddr()) {
                    return CpuStepStateEnum::Exit;
                }
                // dummy read
                core.memory.read_dummy_cross(
                    self.temp_address,
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
