// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct AbsoluteXRMW;

impl CpuStepState for AbsoluteXRMW {
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
                let address_high = core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
                core.internal_stat.set_tempaddr(
                    core.internal_stat.get_tempaddr() | usize::from(address_high) << 8,
                );
                core.internal_stat.set_address(
                    core.internal_stat
                        .get_tempaddr()
                        .wrapping_add(usize::from(core.register.get_x()))
                        & 0xFFFF,
                );
            }
            3 => {
                // dummy read
                core.memory.read_dummy_cross(
                    core.internal_stat.get_tempaddr(),
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            _ => {
                return exit_addressing_mode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
