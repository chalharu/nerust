// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

struct Step1<F: Fn(&mut Register, u8) -> ()> {
    address: usize,
    func: F,
}

impl<F: Fn(&mut Register, u8) -> ()> Step1<F> {
    pub fn new(address: usize, func: F) -> Self {
        Self { address, func }
    }
}

impl<F: Fn(&mut Register, u8) -> ()> CpuStepState for Step1<F> {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let a = core.memory.read(
            self.address,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );

        core.register.set_nz_from_value(a);
        (self.func)(&mut core.register, a);

        FetchOpCode::new(&core.interrupt)
    }
}

pub(crate) struct Lda;
impl OpCode for Lda {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address, Register::set_a))
    }
    fn name(&self) -> &'static str {
        "LDA"
    }
}

pub(crate) struct Ldx;
impl OpCode for Ldx {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address, Register::set_x))
    }
    fn name(&self) -> &'static str {
        "LDX"
    }
}

pub(crate) struct Ldy;
impl OpCode for Ldy {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address, Register::set_y))
    }
    fn name(&self) -> &'static str {
        "LDY"
    }
}
