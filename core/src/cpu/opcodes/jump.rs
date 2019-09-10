// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Jmp;

impl CpuStepState for Jmp {
    fn exec(
        core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut dyn Cartridge,
        _controller: &mut dyn Controller,
        _apu: &mut Apu,
    ) -> CpuStepStateEnum {
        core.register
            .set_pc(core.internal_stat.get_address() as u16);
        exit_opcode(core)
    }
}

pub(crate) struct Jsr;

impl CpuStepState for Jsr {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
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

                core.internal_stat
                    .set_tempaddr(core.register.get_pc().wrapping_sub(1) as usize);
            }
            2 => {
                let hi = (core.internal_stat.get_tempaddr() >> 8) as u8;
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
                let low = (core.internal_stat.get_tempaddr() & 0xFF) as u8;
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
                core.register
                    .set_pc(core.internal_stat.get_address() as u16);
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Rts;

impl CpuStepState for Rts {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
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
                core.internal_stat
                    .set_tempaddr(usize::from(core.memory.read(
                        sp | 0x100,
                        ppu,
                        cartridge,
                        controller,
                        apu,
                        &mut core.interrupt,
                    )));

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
                core.internal_stat
                    .set_tempaddr(core.internal_stat.get_tempaddr() | usize::from(high) << 8);
            }
            5 => {
                core.register
                    .set_pc(core.internal_stat.get_tempaddr() as u16);
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
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
