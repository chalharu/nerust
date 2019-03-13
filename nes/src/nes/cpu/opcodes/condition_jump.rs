// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Bcc;
impl OpCode for Bcc {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        condition_jump(!register.get_c(), address, interrupt)
    }
    fn name(&self) -> &'static str {
        "BCC"
    }
}

pub(crate) struct Bcs;
impl OpCode for Bcs {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        condition_jump(register.get_c(), address, interrupt)
    }
    fn name(&self) -> &'static str {
        "BCS"
    }
}

pub(crate) struct Beq;
impl OpCode for Beq {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        condition_jump(register.get_z(), address, interrupt)
    }
    fn name(&self) -> &'static str {
        "BEQ"
    }
}

pub(crate) struct Bmi;
impl OpCode for Bmi {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        condition_jump(register.get_n(), address, interrupt)
    }
    fn name(&self) -> &'static str {
        "BMI"
    }
}

pub(crate) struct Bne;
impl OpCode for Bne {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        condition_jump(!register.get_z(), address, interrupt)
    }
    fn name(&self) -> &'static str {
        "BNE"
    }
}

pub(crate) struct Bpl;
impl OpCode for Bpl {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        condition_jump(!register.get_n(), address, interrupt)
    }
    fn name(&self) -> &'static str {
        "BPL"
    }
}

pub(crate) struct Bvc;
impl OpCode for Bvc {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        condition_jump(!register.get_v(), address, interrupt)
    }
    fn name(&self) -> &'static str {
        "BVC"
    }
}

pub(crate) struct Bvs;
impl OpCode for Bvs {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        condition_jump(register.get_v(), address, interrupt)
    }
    fn name(&self) -> &'static str {
        "BVS"
    }
}

fn condition_jump(
    condition: bool,
    address: usize,
    interrupt: &mut Interrupt,
) -> Box<dyn CpuStepState> {
    if condition {
        Box::new(Step1::new(address, interrupt.detected))
    } else {
        FetchOpCode::new(interrupt)
    }
}

struct Step1 {
    address: usize,
    interrupt_detected: bool,
}

impl Step1 {
    pub fn new(address: usize, interrupt_detected: bool) -> Self {
        Self {
            address,
            interrupt_detected,
        }
    }
}

impl CpuStepState for Step1 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        // dummy read
        read_dummy_current(core, ppu, cartridge, controller, apu);
        if !self.interrupt_detected {
            core.interrupt.executing = false;
        }

        let pc = core.register.get_pc() as usize;
        if page_crossed(self.address, pc) {
            Box::new(Step2::new(self.address))
        } else {
            core.register.set_pc(self.address as u16);
            FetchOpCode::new(&core.interrupt)
        }
    }
}

struct Step2 {
    address: usize,
}

impl Step2 {
    pub fn new(address: usize) -> Self {
        Self { address }
    }
}

impl CpuStepState for Step2 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        // dummy read
        read_dummy_current(core, ppu, cartridge, controller, apu);

        core.register.set_pc(self.address as u16);
        FetchOpCode::new(&core.interrupt)
    }
}
