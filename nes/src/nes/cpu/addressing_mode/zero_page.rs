// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct ZeroPage {
    step: usize,
}

impl ZeroPage {
    pub fn new() -> Self {
        Self { step: 0 }
    }
}

impl CpuStepState for ZeroPage {
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
                let addr = usize::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.register.set_opaddr(addr);
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
