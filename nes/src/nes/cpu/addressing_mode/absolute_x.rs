// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct AbsoluteX;

impl CpuStepState for AbsoluteX {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.register.get_opstep() {
            1 => {
                let addr = usize::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.register.set_op_tempaddr(addr);
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
                core.register.set_op_tempaddr(
                    core.register.get_op_tempaddr() | usize::from(address_high) << 8,
                );
                core.register.set_opaddr(
                    core.register
                        .get_op_tempaddr()
                        .wrapping_add(usize::from(core.register.get_x()))
                        & 0xFFFF,
                );
            }
            3 => {
                if !page_crossed(core.register.get_op_tempaddr(), core.register.get_opaddr()) {
                    return exit_addressing_mode(core);
                }
                // dummy read
                core.memory.read_dummy_cross(
                    core.register.get_op_tempaddr(),
                    core.register.get_opaddr(),
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
