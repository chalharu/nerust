// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Accumulator;

impl Accumulator {
    pub fn new() -> Self {
        Self
    }
}

impl CpuStepState for Accumulator {
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
