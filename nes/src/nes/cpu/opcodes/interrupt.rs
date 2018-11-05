// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Brk;
impl OpCode for Brk {
    fn next_func(
        &self,
        _address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(BrkStep1::new())
    }
    fn name(&self) -> &'static str {
        "BRK"
    }
}

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
        core.memory.read_next(
            &mut core.register,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );

        Box::new(BrkStep2::new())
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
        let pc = core.register.get_pc();
        let hi = (pc >> 8) as u8;
        let low = (pc & 0xFF) as u8;

        push(core, ppu, cartridge, controller, apu, hi);

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
        push(core, ppu, cartridge, controller, apu, self.low);

        Box::new(BrkStep4::new(if core.interrupt.nmi {
            // core.interrupt.nmi = false;
            NMI_VECTOR
        } else {
            IRQ_VECTOR
        }))
    }
}

struct BrkStep4 {
    vector: usize,
}

impl BrkStep4 {
    pub fn new(vector: usize) -> Self {
        Self { vector }
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
        let p = core.register.get_p() | (RegisterP::Break | RegisterP::Reserved).bits();
        push(core, ppu, cartridge, controller, apu, p);
        Box::new(BrkStep5::new(self.vector))
    }
}

struct BrkStep5 {
    vector: usize,
}

impl BrkStep5 {
    pub fn new(vector: usize) -> Self {
        Self { vector }
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
        let low = core.memory.read(
            self.vector,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        Box::new(BrkStep6::new(self.vector, low))
    }
}

struct BrkStep6 {
    vector: usize,
    low: u8,
}

impl BrkStep6 {
    pub fn new(vector: usize, low: u8) -> Self {
        Self { vector, low }
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
        let hi = u16::from(core.memory.read(
            self.vector + 1,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        ));
        core.register.set_pc((hi << 8) | u16::from(self.low));
        core.interrupt.executing = false;
        FetchOpCode::new(&core.interrupt)
    }
}

pub(crate) struct Rti;
impl OpCode for Rti {
    fn next_func(
        &self,
        _address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(RtiStep1::new())
    }
    fn name(&self) -> &'static str {
        "RTI"
    }
}

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
        read_dummy_current(core, ppu, cartridge, controller, apu);
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
        let _ = core.memory.read(
            sp | 0x100,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );

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
        let p = pull(core, ppu, cartridge, controller, apu);
        core.register
            .set_p((p & !(RegisterP::Break.bits())) | RegisterP::Reserved.bits());

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
        let low = pull(core, ppu, cartridge, controller, apu);

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
        let high = pull(core, ppu, cartridge, controller, apu);

        core.register
            .set_pc(u16::from(self.low) | (u16::from(high) << 8));
        FetchOpCode::new(&core.interrupt)
    }
}

pub(crate) struct Irq {}

impl Irq {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for Irq {
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

        Box::new(IrqStep2::new())
    }
}

struct IrqStep2 {}

impl IrqStep2 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for IrqStep2 {
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

        Box::new(IrqStep3::new())
    }
}

struct IrqStep3 {}

impl IrqStep3 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for IrqStep3 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let pc = core.register.get_pc();
        let hi = (pc >> 8) as u8;
        let low = (pc & 0xFF) as u8;
        push(core, ppu, cartridge, controller, apu, hi);
        Box::new(IrqStep4::new(low))
    }
}

struct IrqStep4 {
    low: u8,
}

impl IrqStep4 {
    pub fn new(low: u8) -> Self {
        Self { low }
    }
}

impl CpuStepState for IrqStep4 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        push(core, ppu, cartridge, controller, apu, self.low);

        Box::new(IrqStep5::new(
            if core.interrupt.nmi {
                NMI_VECTOR
            } else {
                IRQ_VECTOR
            },
            core.interrupt.nmi,
        ))
    }
}

struct IrqStep5 {
    vector: usize,
    nmi: bool,
}

impl IrqStep5 {
    pub fn new(vector: usize, nmi: bool) -> Self {
        Self { vector, nmi }
    }
}

impl CpuStepState for IrqStep5 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let p = (core.register.get_p() & !RegisterP::Break.bits()) | RegisterP::Reserved.bits();
        push(core, ppu, cartridge, controller, apu, p);

        Box::new(IrqStep6::new(self.vector, self.nmi))
    }
}

struct IrqStep6 {
    vector: usize,
    nmi: bool,
}

impl IrqStep6 {
    pub fn new(vector: usize, nmi: bool) -> Self {
        Self { vector, nmi }
    }
}

impl CpuStepState for IrqStep6 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        core.register.set_i(true);
        let low = core.memory.read(
            self.vector,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        if self.nmi {
            core.interrupt.nmi = false;
        }
        Box::new(IrqStep7::new(self.vector, low))
    }
}

struct IrqStep7 {
    vector: usize,
    low: u8,
}

impl IrqStep7 {
    pub fn new(vector: usize, low: u8) -> Self {
        Self { vector, low }
    }
}

impl CpuStepState for IrqStep7 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let hi = u16::from(core.memory.read(
            self.vector + 1,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        ));
        core.register.set_pc((hi << 8) | u16::from(self.low));
        core.interrupt.executing = false;
        FetchOpCode::new(&core.interrupt)
    }
}

pub(crate) struct Reset {}

impl Reset {
    pub fn new() -> Box<dyn CpuStepState> {
        Box::new(Self {})
    }
}

impl CpuStepState for Reset {
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

        core.interrupt.irq_flag = IrqSource::empty();
        core.interrupt.irq_mask = IrqSource::All;
        core.interrupt.nmi = false;

        Box::new(ResetStep2::new())
    }
}

struct ResetStep2 {}

impl ResetStep2 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for ResetStep2 {
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
        Box::new(ResetStep3::new())
    }
}

struct ResetStep3 {}

impl ResetStep3 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for ResetStep3 {
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
        core.memory.read(
            0x100 | sp,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        Box::new(ResetStep4::new())
    }
}

struct ResetStep4 {}

impl ResetStep4 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for ResetStep4 {
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
        core.memory.read(
            0x100 | sp,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );

        Box::new(ResetStep5::new())
    }
}

struct ResetStep5 {}

impl ResetStep5 {
    pub fn new() -> Self {
        Self {}
    }
}

impl CpuStepState for ResetStep5 {
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
        core.memory.read(
            0x100 | sp,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );

        Box::new(ResetStep6::new(RESET_VECTOR))
    }
}

struct ResetStep6 {
    vector: usize,
}

impl ResetStep6 {
    pub fn new(vector: usize) -> Self {
        Self { vector }
    }
}

impl CpuStepState for ResetStep6 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        core.register.set_i(true);
        let low = core.memory.read(
            self.vector,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        Box::new(ResetStep7::new(self.vector, low))
    }
}

struct ResetStep7 {
    vector: usize,
    low: u8,
}

impl ResetStep7 {
    pub fn new(vector: usize, low: u8) -> Self {
        Self { vector, low }
    }
}

impl CpuStepState for ResetStep7 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let hi = u16::from(core.memory.read(
            self.vector + 1,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        ));
        core.register.set_pc((hi << 8) | u16::from(self.low));
        core.interrupt.executing = false;
        FetchOpCode::new(&core.interrupt)
    }
}
