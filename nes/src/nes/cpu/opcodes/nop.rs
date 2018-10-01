// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Nop;
impl OpCode for Nop {
    fn next_func(
        &self,
        _address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step2::new())
    }
    fn name(&self) -> &'static str {
        "NOP"
    }
}

pub(crate) struct Kil;
impl OpCode for Kil {
    fn next_func(
        &self,
        _address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new())
    }
    fn name(&self) -> &'static str {
        "KIL"
    }
}

struct Step1 {}

impl Step1 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for Step1 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let pc = core.register.get_pc() as usize;
        let _ = core.memory.read(pc, ppu, cartridge, controller, apu, &mut core.interrupt);
        Box::new(Step2::new())
    }
}

struct Step2 {}

impl Step2 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for Step2 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let pc = core.register.get_pc() as usize;
        let _ = core.memory.read(pc, ppu, cartridge, controller, apu, &mut core.interrupt);
        FetchOpCode::new(&core.interrupt)
    }
}
