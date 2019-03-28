// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) trait Arithmetic: CpuStep {
    fn calculator(register: &mut Register, a: u8, b: u8) -> u8;

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
                let data = core.memory.read(
                    core.register.get_opaddr(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
                let a = core.register.get_a();
                let result = Self::calculator(&mut core.register, a, data);

                core.register.set_nz_from_value(result);
                core.register.set_a(result);
            }
            _ => {
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }
}

macro_rules! arithmetic {
    ($name:ident, $calc:expr) => {
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

        impl Arithmetic for $name {
            fn calculator(register: &mut Register, a: u8, b: u8) -> u8 {
                $calc(register, a, b)
            }
        }

        cpu_step_state_impl!($name);
    };
}

arithmetic!(And, |_register: &mut Register, a: u8, b: u8| a & b);
arithmetic!(Eor, |_register: &mut Register, a: u8, b: u8| a ^ b);
arithmetic!(Ora, |_register: &mut Register, a: u8, b: u8| a | b);
arithmetic!(Adc, |register: &mut Register, a_u8: u8, b_u8: u8| {
    let a = u16::from(a_u8);
    let b = u16::from(b_u8);
    let c = if register.get_c() { 1 } else { 0 };
    let d = a + b + c;
    register.set_c(d > 0xFF);
    register.set_v((a ^ b) & 0x80 == 0 && (a ^ d) & 0x80 != 0);
    (d & 0xFF) as u8
});
arithmetic!(Sbc, |register: &mut Register, a_u8: u8, b_u8: u8| {
    let a = u16::from(a_u8);
    let b = u16::from(b_u8);
    let c = if register.get_c() { 0 } else { 1 };
    let d = a.wrapping_sub(b).wrapping_sub(c);
    register.set_c(d <= 0xFF);
    register.set_v((a ^ b) & 0x80 != 0 && (a ^ d) & 0x80 != 0);
    (d & 0xFF) as u8
});
