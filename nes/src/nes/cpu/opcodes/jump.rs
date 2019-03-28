// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Jmp {}

impl Jmp {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for Jmp {
    fn entry(
        &mut self,
        core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) {
        core.register.set_pc(core.register.get_opaddr() as u16);
    }

    fn exec(
        &mut self,
        _core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) -> CpuStepStateEnum {
        CpuStepStateEnum::Exit
    }
}

pub(crate) struct Jsr {
    step: usize,
    data: u16,
}

impl Jsr {
    pub fn new() -> Self {
        Self { step: 0, data: 0 }
    }
}

impl CpuStepState for Jsr {
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
                let sp = usize::from(core.register.get_sp());
                let _ = core.memory.read(
                    0x100 | sp,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );

                self.data = core.register.get_pc().wrapping_sub(1);
            }
            2 => {
                let hi = (self.data >> 8) as u8;
                let sp = usize::from(core.register.get_sp());
                core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
                core.memory.write(
                    0x100 | sp,
                    hi,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            3 => {
                let low = (self.data & 0xFF) as u8;
                let sp = usize::from(core.register.get_sp());
                core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
                core.memory.write(
                    0x100 | sp,
                    low,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
                core.register.set_pc(core.register.get_opaddr() as u16);
            }
            _ => {
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Rts {
    step: usize,
    data: u16,
}

impl Rts {
    pub fn new() -> Self {
        Self { step: 0, data: 0 }
    }
}

impl CpuStepState for Rts {
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
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);
            }
            2 => {
                // dummy read
                let sp = usize::from(core.register.get_sp());
                let _ = core.memory.read(
                    sp | 0x100,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );

                core.register.set_sp((sp.wrapping_add(1) & 0xFF) as u8);
            }
            3 => {
                let sp = usize::from(core.register.get_sp());
                self.data = u16::from(core.memory.read(
                    sp | 0x100,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));

                core.register.set_sp((sp.wrapping_add(1) & 0xFF) as u8);
            }
            4 => {
                let sp = usize::from(core.register.get_sp());
                let high = core.memory.read(
                    sp | 0x100,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
                self.data |= u16::from(high) << 8;
            }
            5 => {
                core.register.set_pc(self.data);
                core.memory.read_next(
                    &mut core.register,
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
}
