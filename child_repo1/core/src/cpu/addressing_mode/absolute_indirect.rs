// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct AbsoluteIndirect;

impl CpuStepState for AbsoluteIndirect {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let addr = usize::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.internal_stat.set_tempaddr(addr);
            }
            2 => {
                let address_high = usize::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.internal_stat
                    .set_address((address_high << 8) | core.internal_stat.get_tempaddr());
            }
            3 => {
                let addr = usize::from(core.memory.read(
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.internal_stat.set_tempaddr(addr);
            }
            4 => {
                let address_high = usize::from(core.memory.read(
                    (core.internal_stat.get_address().wrapping_add(1) & 0xFF)
                        | (core.internal_stat.get_address() & 0xFF00),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.internal_stat
                    .set_address((address_high << 8) | core.internal_stat.get_tempaddr());
            }
            _ => {
                return exit_addressing_mode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
