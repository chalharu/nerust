// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

struct Step1<FCalc: Fn(&mut Register, u8, u8) -> u8> {
    address: usize,
    calculator: FCalc,
}

impl<FCalc: Fn(&mut Register, u8, u8) -> u8> Step1<FCalc> {
    pub fn new(address: usize, calculator: FCalc) -> Self {
        Self {
            address,
            calculator,
        }
    }
}

impl<FCalc: Fn(&mut Register, u8, u8) -> u8> CpuStepState for Step1<FCalc> {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let data = core
            .memory
            .read(self.address, ppu, cartridge, controller, apu, &mut core.interrupt);
        let a = core.register.get_a();
        let result = (self.calculator)(&mut core.register, a, data);

        core.register.set_nz_from_value(result);
        core.register.set_a(result);

        FetchOpCode::new(&core.interrupt)
    }
}

pub(crate) struct And;
impl OpCode for And {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address, |_register, a, data| a & data))
    }
    fn name(&self) -> &'static str {
        "AND"
    }
}

pub(crate) struct Eor;
impl OpCode for Eor {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address, |_register, a, data| a ^ data))
    }
    fn name(&self) -> &'static str {
        "EOR"
    }
}

pub(crate) struct Ora;
impl OpCode for Ora {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address, |_register, a, data| a | data))
    }
    fn name(&self) -> &'static str {
        "ORA"
    }
}

pub(crate) struct Adc;
impl OpCode for Adc {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address, |register, a_u8, b_u8| {
            let a = u16::from(a_u8);
            let b = u16::from(b_u8);
            let c = if register.get_c() { 1 } else { 0 };
            let d = a + b + c;
            register.set_c(d > 0xFF);
            register.set_v((a ^ b) & 0x80 == 0 && (a ^ d) & 0x80 != 0);
            (d & 0xFF) as u8
        }))
    }
    fn name(&self) -> &'static str {
        "ADC"
    }
}

pub(crate) struct Sbc;
impl OpCode for Sbc {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(Step1::new(address, |register, a_u8, b_u8| {
            let a = u16::from(a_u8);
            let b = u16::from(b_u8);
            let c = if register.get_c() { 0 } else { 1 };
            let d = a.wrapping_sub(b).wrapping_sub(c);
            register.set_c(d <= 0xFF);
            register.set_v((a ^ b) & 0x80 != 0 && (a ^ d) & 0x80 != 0);
            (d & 0xFF) as u8
        }))
    }
    fn name(&self) -> &'static str {
        "SBC"
    }
}
