// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) trait Compare: CpuStep {
    fn comparer(register: &Register) -> u8;

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
                let a = Self::comparer(&core.register);
                let b = core.memory.read(
                    core.register.get_opaddr(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );

                core.register.set_nz_from_value(a.wrapping_sub(b));
                core.register.set_c(a >= b);
            }
            _ => {
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }
}

macro_rules! compare {
    ($name:ident, $comparer:expr) => {
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

        impl Compare for $name {
            fn comparer(register: &Register) -> u8 {
                $comparer(register)
            }
        }

        cpu_step_state_impl!($name);
    };
}

compare!(Cmp, Register::get_a);
compare!(Cpx, Register::get_x);
compare!(Cpy, Register::get_y);
