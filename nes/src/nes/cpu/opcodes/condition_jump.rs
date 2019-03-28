// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

macro_rules! condition_jump {
    ($name:ident, $cond:expr) => {
        pub(crate) struct $name {
            step: usize,
        }

        impl $name {
            pub fn new() -> Self {
                Self { step: 0 }
            }
        }

        impl CpuStep for $name {
            fn get_step(&self) -> usize {
                self.step
            }

            fn set_step(&mut self, value: usize) {
                self.step = value;
            }
        }

        impl ConditionJump for $name {
            fn condition(register: &Register) -> bool {
                $cond(register)
            }
        }

        cpu_step_state_impl!($name);
    };
}

condition_jump!(Bcc, |r: &Register| !r.get_c());
condition_jump!(Bcs, Register::get_c);
condition_jump!(Beq, Register::get_z);
condition_jump!(Bmi, Register::get_n);
condition_jump!(Bne, |r: &Register| !r.get_z());
condition_jump!(Bpl, |r: &Register| !r.get_n());
condition_jump!(Bvc, |r: &Register| !r.get_v());
condition_jump!(Bvs, Register::get_v);

pub(crate) trait ConditionJump: CpuStep {
    fn condition(register: &Register) -> bool;

    fn entry_opcode(
        &mut self,
        _core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) {
        self.set_step(0);
    }

    fn exec_opcode(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        let step = self.get_step() + 1;
        self.set_step(step);
        match step {
            1 => {
                if !Self::condition(&core.register) {
                    return CpuStepStateEnum::Exit;
                }
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);
                if !core.interrupt.detected {
                    core.interrupt.executing = false;
                }
            }
            2 => {
                let pc = core.register.get_pc() as usize;
                if !page_crossed(core.register.get_opaddr(), pc) {
                    core.register.set_pc(core.register.get_opaddr() as u16);
                    return CpuStepStateEnum::Exit;
                }
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);

                core.register.set_pc(core.register.get_opaddr() as u16);
            }
            _ => {
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }
}
