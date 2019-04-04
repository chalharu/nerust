// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) trait Load {
    fn setter(register: &mut Register, value: u8);

    fn exec_opcode(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.register.get_opstep() {
            1 => {
                let a = core.memory.read(
                    core.register.get_opaddr(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );

                core.register.set_nz_from_value(a);
                Self::setter(&mut core.register, a);
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

macro_rules! load {
    ($name:ident, $func:expr) => {
        pub(crate) struct $name;

        impl Load for $name {
            fn setter(register: &mut Register, value: u8) {
                ($func)(register, value);
            }
        }

        cpu_step_state_impl!($name);
    };
}

load!(Lda, Register::set_a);
load!(Ldx, Register::set_x);
load!(Ldy, Register::set_y);
