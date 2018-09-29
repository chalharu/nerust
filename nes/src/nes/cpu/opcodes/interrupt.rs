// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Brk;
impl OpCode for Brk {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(BrkStep1::new())
    }
    fn name(&self) -> &'static str {
        "BRK"
    }
}

// struct Brk;
// impl OpCode for Brk {
//     fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
//         let pc = state.register().get_pc().wrapping_add(1);
//         // state.register().set_b(true);
//         push_u16(state, memory, pc);
//         Php.execute(state, memory, address);
//         state.register().set_i(true);
//         state.interrupt.started = InterruptStatus::Executing;
//         3
//     }
//     fn name(&self) -> &'static str {
//         "BRK"
//     }
// }

struct BrkStep1 {}

impl BrkStep1 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for BrkStep1 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        // dummy read
        core.memory
            .read_next(&mut core.register, ppu, cartridge, controller, apu);

        if core.register.get_i() {
            Box::new(FetchOpCode::new())
        } else {
            core.register.set_b(true);
            Box::new(BrkStep2::new())
        }
    }
}

struct BrkStep2 {}

impl BrkStep2 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for BrkStep2 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let pc = core.register.get_pc().wrapping_add(1);
        let hi = (pc >> 8) as u8;
        let low = (pc & 0xFF) as u8;
        let sp = usize::from(core.register.get_sp());
        core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
        core.memory
            .write(0x100 | sp, hi, ppu, cartridge, controller, apu);
        Box::new(BrkStep3::new(low))
    }
}

struct BrkStep3 {
    low: u8,
}

impl BrkStep3 {
    pub fn new(low: u8) -> Self {
        Self { low }
    }
}

impl CpuStepState for BrkStep3 {
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
            .write(0x100 | sp, self.low, ppu, cartridge, controller, apu);

        Box::new(BrkStep4::new())
    }
}

struct BrkStep4 {}

impl BrkStep4 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for BrkStep4 {
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
        let p = core.register.get_p() | 0x10;
        core.memory
            .write(0x100 | sp, p, ppu, cartridge, controller, apu);

        Box::new(BrkStep5::new())
    }
}

struct BrkStep5 {}

impl BrkStep5 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for BrkStep5 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        core.register.set_i(true);
        let low = core.memory.read(0xFFFE, ppu, cartridge, controller, apu);
        Box::new(BrkStep6::new(low))
    }
}

struct BrkStep6 {
    low: u8,
}

impl BrkStep6 {
    pub fn new(low: u8) -> Self {
        Self { low }
    }
}

impl CpuStepState for BrkStep6 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let hi = u16::from(core.memory.read(0xFFFF, ppu, cartridge, controller, apu));
        core.register.set_pc((hi << 8) | u16::from(self.low));
        Box::new(FetchOpCode::new())
    }
}
pub(crate) struct Rti;
impl OpCode for Rti {
    fn next_func(&self, address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(RtiStep1::new())
    }
    fn name(&self) -> &'static str {
        "RTI"
    }
}

// struct Rti;
// impl OpCode for Rti {
//     fn execute(&self, state: &mut State, memory: &mut Memory, _address: usize) -> usize {
//         let result = pull(state, memory);
//         state.register().set_p((result & 0xEF) | 0x20);
//         let result = pull_u16(state, memory);
//         state.register().set_pc(result);
//         // 割り込み検出
//         if state.interrupt.get_irq() && !state.register().get_i() {
//             // state.interrupt.irq = false;
//             state.interrupt.started = InterruptStatus::Detected;
//         }
//         5
//     }
//     fn name(&self) -> &'static str {
//         "RTI"
//     }
// }

struct RtiStep1 {}

impl RtiStep1 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for RtiStep1 {
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
        let _ = core.memory.read(pc, ppu, cartridge, controller, apu);
        Box::new(RtiStep2::new())
    }
}

struct RtiStep2 {}

impl RtiStep2 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for RtiStep2 {
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
            .read(sp | 0x100, ppu, cartridge, controller, apu);

        core.register.set_sp((sp.wrapping_add(1) & 0xFF) as u8);

        Box::new(RtiStep3::new())
    }
}

struct RtiStep3 {}

impl RtiStep3 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for RtiStep3 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let sp = usize::from(core.register.get_sp());
        let p = core
            .memory
            .read(sp | 0x100, ppu, cartridge, controller, apu);

        core.register.set_p((p & 0xEF) | 0x20);
        core.register.set_sp((sp.wrapping_add(1) & 0xFF) as u8);

        Box::new(RtiStep4::new())
    }
}

struct RtiStep4 {}

impl RtiStep4 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for RtiStep4 {
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
            .read(sp | 0x100, ppu, cartridge, controller, apu);

        core.register.set_sp((sp.wrapping_add(1) & 0xFF) as u8);

        Box::new(RtiStep5::new(low))
    }
}

struct RtiStep5 {
    low: u8,
}

impl RtiStep5 {
    pub fn new(low: u8) -> Self {
        Self { low }
    }
}

impl CpuStepState for RtiStep5 {
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
            .read(sp | 0x100, ppu, cartridge, controller, apu);

        core.register
            .set_pc(u16::from(self.low) | (u16::from(high) << 8));
        Box::new(FetchOpCode::new())
    }
}
