// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Clc;
impl OpCode for Clc {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(|r| r.set_c(false)))
    }
    fn name(&self) -> &'static str {
        "CLC"
    }
}

pub(crate) struct Cld;
impl OpCode for Cld {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(|r| r.set_d(false)))
    }
    fn name(&self) -> &'static str {
        "CLD"
    }
}

pub(crate) struct Cli;
impl OpCode for Cli {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(|r| r.set_i(false)))
    }
    fn name(&self) -> &'static str {
        "CLI"
    }
}

pub(crate) struct Clv;
impl OpCode for Clv {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(|r| r.set_v(false)))
    }
    fn name(&self) -> &'static str {
        "CLV"
    }
}

pub(crate) struct Sec;
impl OpCode for Sec {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(|r| r.set_c(true)))
    }
    fn name(&self) -> &'static str {
        "SEC"
    }
}

pub(crate) struct Sed;
impl OpCode for Sed {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(|r| r.set_d(true)))
    }
    fn name(&self) -> &'static str {
        "SED"
    }
}

pub(crate) struct Sei;
impl OpCode for Sei {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(|r| r.set_i(true)))
    }
    fn name(&self) -> &'static str {
        "SEI"
    }
}

struct Step1<F: Fn(&mut Register) -> ()> {
    func: F,
}

impl<F: Fn(&mut Register) -> ()> Step1<F> {
    pub fn new(func: F) -> Self {
        Self { func }
    }
}

impl<F: Fn(&mut Register) -> ()> CpuStepState for Step1<F> {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        // dummy read
        read_dummy_current(core, ppu, cartridge, controller, apu);

        (self.func)(&mut core.register);
        FetchOpCode::new(&core.interrupt)
    }
}
