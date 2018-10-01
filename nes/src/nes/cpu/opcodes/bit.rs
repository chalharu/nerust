// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Bit;
impl OpCode for Bit {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address))
    }
    fn name(&self) -> &'static str {
        "BIT"
    }
}

struct Step1 {
    address: usize,
}

impl Step1 {
    pub fn new(address: usize) -> Self {
        Self { address }
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
        let data = core
            .memory
            .read(self.address, ppu, cartridge, controller, apu, &mut core.interrupt);
        let a = data & core.register.get_a();
        core.register.set_z_from_value(a);
        core.register.set_n_from_value(data);

        FetchOpCode::new(&core.interrupt)
    }
}
