// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct IndexedIndirect;
impl AddressingMode for IndexedIndirect {
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
        "IndexedIndirect"
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
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let pc = core.register.get_pc() as usize;
        let address_low =
            core.memory
                .read(pc, ppu, cartridge, controller, apu, &mut core.interrupt);
        Box::new(Step2::new(self.code, address_low))
    }
}

struct Step2 {
    code: usize,
    address_low: u8,
}

impl Step2 {
    pub fn new(code: usize, address_low: u8) -> Self {
        Self { code, address_low }
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
        let _ = core.memory.read_next(
            &mut core.register,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        let zeropage_address = self.address_low.wrapping_add(core.register.get_x());
        Box::new(Step3::new(self.code, usize::from(zeropage_address)))
    }
}

struct Step3 {
    code: usize,
    zeropage_address: usize,
}

impl Step3 {
    pub fn new(code: usize, zeropage_address: usize) -> Self {
        Self {
            code,
            zeropage_address,
        }
    }
}

impl CpuStepState for Step3 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let address_low = core.memory.read(
            self.zeropage_address,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        Box::new(Step4::new(self.code, self.zeropage_address, address_low))
    }
}

struct Step4 {
    code: usize,
    zeropage_address: usize,
    address_low: u8,
}

impl Step4 {
    pub fn new(code: usize, zeropage_address: usize, address_low: u8) -> Self {
        Self {
            code,
            zeropage_address,
            address_low,
        }
    }
}

impl CpuStepState for Step4 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let address_high = usize::from(core.memory.read(
            self.zeropage_address.wrapping_add(1) & 0xFF,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        ));
        let new_address = (address_high << 8) | usize::from(self.address_low);
        core.opcode_tables.get(self.code).next_func(
            new_address,
            &mut core.register,
            &mut core.interrupt,
        )
    }
}
