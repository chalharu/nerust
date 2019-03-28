// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) trait FlagControl: CpuStep {
    fn setter(register: &mut Register);

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
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);

                Self::setter(&mut core.register);
            }
            _ => {
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }
}

macro_rules! flag_control {
    ($name:ident, $func:expr) => {
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

        impl FlagControl for $name {
            fn setter(register: &mut Register) {
                $func(register);
            }
        }

        cpu_step_state_impl!($name);
    };
}

flag_control!(Clc, |r: &mut Register| r.set_c(false));
flag_control!(Cld, |r: &mut Register| r.set_d(false));
flag_control!(Cli, |r: &mut Register| r.set_i(false));
flag_control!(Clv, |r: &mut Register| r.set_v(false));
flag_control!(Sec, |r: &mut Register| r.set_c(true));
flag_control!(Sed, |r: &mut Register| r.set_d(true));
flag_control!(Sei, |r: &mut Register| r.set_i(true));
