// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Jmp;
impl OpCode for Jmp {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        register.set_pc(address as u16);
        FetchOpCode::new(interrupt)
    }
    fn name(&self) -> &'static str {
        "JMP"
    }
}

pub(crate) struct Jsr;
impl OpCode for Jsr {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(JsrStep1::new(address))
    }
    fn name(&self) -> &'static str {
        "JSR"
    }
}

struct JsrStep1 {
    address: usize,
}

impl JsrStep1 {
    pub fn new(address: usize) -> Self {
        Self { address }
    }
}

impl CpuStepState for JsrStep1 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        // dummy read
        let sp = usize::from(core.register.get_sp());
        let _ = core
            .memory
            .read(0x100 | sp, ppu, cartridge, controller, apu, &mut core.interrupt);

        let pc = core.register.get_pc().wrapping_sub(1);
        Box::new(JsrStep2::new(self.address, pc))
    }
}

struct JsrStep2 {
    address: usize,
    pc: u16,
}

impl JsrStep2 {
    pub fn new(address: usize, pc: u16) -> Self {
        Self { address, pc }
    }
}

impl CpuStepState for JsrStep2 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let hi = (self.pc >> 8) as u8;
        let low = (self.pc & 0xFF) as u8;
        let sp = usize::from(core.register.get_sp());
        core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
        core.memory
            .write(0x100 | sp, hi, ppu, cartridge, controller, apu, &mut core.interrupt);
        Box::new(JsrStep3::new(self.address, low))
    }
}

struct JsrStep3 {
    address: usize,
    low: u8,
}

impl JsrStep3 {
    pub fn new(address: usize, low: u8) -> Self {
        Self { address, low }
    }
}

impl CpuStepState for JsrStep3 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let sp = usize::from(core.register.get_sp());
        core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
        core.memory
            .write(0x100 | sp, self.low, ppu, cartridge, controller, apu, &mut core.interrupt);
        core.register.set_pc(self.address as u16);
        FetchOpCode::new(&core.interrupt)
    }
}

pub(crate) struct Rts;
impl OpCode for Rts {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(RtsStep1::new())
    }
    fn name(&self) -> &'static str {
        "RTS"
    }
}

struct RtsStep1 {}

impl RtsStep1 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for RtsStep1 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        // dummy read
        let pc = usize::from(core.register.get_pc());
        let _ = core.memory.read(pc, ppu, cartridge, controller, apu, &mut core.interrupt);
        Box::new(RtsStep2::new())
    }
}

struct RtsStep2 {}

impl RtsStep2 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for RtsStep2 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        // dummy read
        let sp = usize::from(core.register.get_sp());
        let _ = core
            .memory
            .read(sp | 0x100, ppu, cartridge, controller, apu, &mut core.interrupt);

        core.register.set_sp((sp.wrapping_add(1) & 0xFF) as u8);

        Box::new(RtsStep3::new())
    }
}

struct RtsStep3 {}

impl RtsStep3 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for RtsStep3 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let sp = usize::from(core.register.get_sp());
        let low = core
            .memory
            .read(sp | 0x100, ppu, cartridge, controller, apu, &mut core.interrupt);

        core.register.set_sp((sp.wrapping_add(1) & 0xFF) as u8);

        Box::new(RtsStep4::new(low))
    }
}

struct RtsStep4 {
    low: u8,
}

impl RtsStep4 {
    pub fn new(low: u8) -> Self {
        Self { low }
    }
}

impl CpuStepState for RtsStep4 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let sp = usize::from(core.register.get_sp());
        let high = core
            .memory
            .read(sp | 0x100, ppu, cartridge, controller, apu, &mut core.interrupt);

        Box::new(RtsStep5::new(u16::from(self.low) | (u16::from(high) << 8)))
    }
}

struct RtsStep5 {
    address: u16,
}

impl RtsStep5 {
    pub fn new(address: u16) -> Self {
        Self { address }
    }
}

impl CpuStepState for RtsStep5 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        core.register.set_pc(self.address);
        core.memory
            .read_next(&mut core.register, ppu, cartridge, controller, apu, &mut core.interrupt);
        FetchOpCode::new(&core.interrupt)
    }
}
