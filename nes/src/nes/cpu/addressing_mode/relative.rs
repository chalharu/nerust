// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Relative;
impl AddressingMode for Relative {
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
        "Relative"
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
        let offset = u16::from(core.memory.read_next(
            &mut core.register,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        ));
        let pc = core.register.get_pc();
        let address = pc
            .wrapping_add(offset)
            .wrapping_sub(if offset < 0x80 { 0 } else { 0x100 });

        core.opcode_tables.get(self.code).next_func(
            address as usize,
            &mut core.register,
            &mut core.interrupt,
        )
        // if page_crossed(pc, address) {
        //     Box::new(Step2::new(self.code, pc, address))
        // } else {
        //     opcodes.get(code).next_func(address)
        // }
    }
}

// struct Step2 {
//     code: usize,
//     address: usize,
//     new_address: usize,
// }

// impl Step2 {
//     pub fn new(code: usize, address: usize, new_address: usize) -> Self {
//         Self {
//             code,
//             address,
//             new_address,
//         }
//     }
// }

// impl CpuStepState for Step2 {
//     fn next(
//         &mut self,
//         core: &mut Core,
//         ppu: &mut Ppu,
//         cartridge: &mut Cartridge,
//         controller: &mut Controller,
//         apu: &mut Apu,
//     ) -> Box<dyn CpuStepState> {
//         // dummy read
//         core.memory.read_dummy(
//             self.address,
//             self.new_address,
//             ppu,
//             cartridge,
//             controller,
//             apu,
//         );
//         core.opcode_tables
//             .get(self.code)
//             .next_func(self.new_address)
//     }
// }
