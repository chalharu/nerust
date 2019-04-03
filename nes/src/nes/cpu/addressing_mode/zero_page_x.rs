// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct ZeroPageX {
    temp_address: usize,
}

impl ZeroPageX {
    pub fn new() -> Self {
        Self { temp_address: 0 }
    }
}

impl CpuStepState for ZeroPageX {
    fn exec(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.register.get_opstep() {
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
                let pc = usize::from(core.register.get_pc());
                core.memory.read_dummy_cross(
                    pc,
                    self.temp_address,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
                core.register.set_opaddr(
                    (self
                        .temp_address
                        .wrapping_add(usize::from(core.register.get_x())))
                        & 0xFF,
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
