// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct ZeroPageY;
impl AddressingMode for ZeroPageY {
    fn next_func(
        &self,
        code: usize,
        _register: &mut Register,
        _opcodes: &mut Opcodes,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(code))
    }

    fn name(&self) -> &'static str {
        "ZeroPageY"
    }
}

struct Step1 {
    code: usize,
}

impl Step1 {
    pub fn new(code: usize) -> Self {
        Self { code }
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
        let zeropage_address =
            usize::from(
                core.memory
                    .read_next(&mut core.register, ppu, cartridge, controller, apu, &mut core.interrupt),
            );

        Box::new(Step2::new(self.code, zeropage_address))
    }
}

struct Step2 {
    code: usize,
    zeropage_address: usize,
}

impl Step2 {
    pub fn new(code: usize, zeropage_address: usize) -> Self {
        Self {
            code,
            zeropage_address,
        }
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
        let pc = usize::from(core.register.get_pc());
        core.memory
            .read_dummy(pc, self.zeropage_address, ppu, cartridge, controller, apu, &mut core.interrupt);
        let address = (self
            .zeropage_address
            .wrapping_add(usize::from(core.register.get_y())))
            & 0xFF;
        core.opcode_tables.get(self.code).next_func(
            address,
            &mut core.register,
            &mut core.interrupt,
        )
    }
}
