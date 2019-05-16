// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct IndexedIndirect;

impl CpuStepState for IndexedIndirect {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let pc = core.register.get_pc() as usize;
                core.internal_stat.set_data(core.memory.read(
                    pc,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
            }
            2 => {
                let _ = core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
                core.internal_stat.set_address(usize::from(
                    core.internal_stat
                        .get_data()
                        .wrapping_add(core.register.get_x()),
                ));
            }
            3 => {
                core.internal_stat.set_data(core.memory.read(
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
            }
            4 => {
                let address_high = usize::from(core.memory.read(
                    core.internal_stat.get_address().wrapping_add(1) & 0xFF,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.internal_stat
                    .set_address((address_high << 8) | usize::from(core.internal_stat.get_data()));
            }
            _ => {
                return exit_addressing_mode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
