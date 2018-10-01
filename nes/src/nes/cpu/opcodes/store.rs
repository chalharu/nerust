// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

struct Step1<F: Fn(&mut Register) -> u8> {
    address: usize,
    func: F,
}

impl<F: Fn(&mut Register) -> u8> Step1<F> {
    pub fn new(address: usize, func: F) -> Self {
        Self { address, func }
    }
}

impl<F: Fn(&mut Register) -> u8> CpuStepState for Step1<F> {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let data = (self.func)(&mut core.register);
        core.memory.write(
            self.address,
            data,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        FetchOpCode::new(&core.interrupt)
    }
}

pub(crate) struct Sta;
impl OpCode for Sta {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address, |r| r.get_a()))
    }
    fn name(&self) -> &'static str {
        "STA"
    }
}

pub(crate) struct Stx;
impl OpCode for Stx {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address, |r| r.get_x()))
    }
    fn name(&self) -> &'static str {
        "STX"
    }
}

pub(crate) struct Sty;
impl OpCode for Sty {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address, |r| r.get_y()))
    }
    fn name(&self) -> &'static str {
        "STY"
    }
}
