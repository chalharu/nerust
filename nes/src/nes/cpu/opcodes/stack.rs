// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

struct PullStep1<F: Fn(&mut Register, u8) -> ()> {
    func: Option<F>,
}

impl<F: Fn(&mut Register, u8) -> ()> PullStep1<F> {
    pub fn new(func: F) -> Self {
        Self { func: Some(func) }
    }
}

impl<F: 'static + Fn(&mut Register, u8) -> ()> CpuStepState for PullStep1<F> {
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
        Box::new(PullStep2::new(
            std::mem::replace(&mut self.func, None).unwrap(),
        ))
    }
}

struct PullStep2<F: Fn(&mut Register, u8) -> ()> {
    func: Option<F>,
}

impl<F: Fn(&mut Register, u8) -> ()> PullStep2<F> {
    pub fn new(func: F) -> Self {
        Self { func: Some(func) }
    }
}

impl<F: 'static + Fn(&mut Register, u8) -> ()> CpuStepState for PullStep2<F> {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        // dummy read
        let sp = core.register.get_sp();
        let _ = core.memory.read(
            0x100 | usize::from(sp),
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );

        Box::new(PullStep3::new(
            std::mem::replace(&mut self.func, None).unwrap(),
        ))
    }
}

struct PullStep3<F: Fn(&mut Register, u8) -> ()> {
    func: F,
}

impl<F: Fn(&mut Register, u8) -> ()> PullStep3<F> {
    pub fn new(func: F) -> Self {
        Self { func }
    }
}

impl<F: Fn(&mut Register, u8) -> ()> CpuStepState for PullStep3<F> {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let value = pull(core, ppu, cartridge, controller, apu);
        (self.func)(&mut core.register, value);
        FetchOpCode::new(&core.interrupt)
    }
}

pub(crate) struct Pla;
impl OpCode for Pla {
    fn next_func(
        &self,
        _address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(PullStep1::new(|r, v| {
            r.set_a(v);
            r.set_nz_from_value(v);
        }))
    }
    fn name(&self) -> &'static str {
        "PLA"
    }
}
pub(crate) struct Plp;
impl OpCode for Plp {
    fn next_func(
        &self,
        _address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(PullStep1::new(|r, v| {
            r.set_p((v & !(RegisterP::BREAK.bits())) | RegisterP::RESERVED.bits())
        }))
    }
    fn name(&self) -> &'static str {
        "PLP"
    }
}

struct PushStep1<F: Fn(&Register) -> u8> {
    func: F,
}

impl<F: Fn(&Register) -> u8> PushStep1<F> {
    pub fn new(func: F) -> Self {
        Self { func }
    }
}

impl<F: Fn(&Register) -> u8> CpuStepState for PushStep1<F> {
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
        let data = (self.func)(&core.register);

        Box::new(PushStep2::new(data))
    }
}

struct PushStep2 {
    data: u8,
}

impl PushStep2 {
    pub fn new(data: u8) -> Self {
        Self { data }
    }
}

impl CpuStepState for PushStep2 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        push(core, ppu, cartridge, controller, apu, self.data);

        FetchOpCode::new(&core.interrupt)
    }
}

pub(crate) struct Pha;
impl OpCode for Pha {
    fn next_func(
        &self,
        _address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(PushStep1::new(Register::get_a))
    }
    fn name(&self) -> &'static str {
        "PHA"
    }
}
pub(crate) struct Php;
impl OpCode for Php {
    fn next_func(
        &self,
        _address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(PushStep1::new(|r| r.get_p() | 0x10))
    }
    fn name(&self) -> &'static str {
        "PHP"
    }
}
