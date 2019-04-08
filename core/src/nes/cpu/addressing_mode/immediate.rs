// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Immediate;

impl CpuStepState for Immediate {
    fn exec(
        core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) -> CpuStepStateEnum {
        let pc = core.register.get_pc();
        core.register.set_pc(pc.wrapping_add(1));
        core.internal_stat.set_address(usize::from(pc));
        exit_addressing_mode(core)
    }
}
