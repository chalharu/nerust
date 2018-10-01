// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

fn decrement(register: &mut Register, data: u8) -> u8 {
    let result = data.wrapping_sub(1);
    register.set_nz_from_value(result);
    result
}

pub(crate) struct Dex;
impl OpCode for Dex {
    fn next_func(
        &self,
        _address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(
            |register| register.get_x(),
            |register, data| register.set_x(data),
            decrement,
        ))
    }
    fn name(&self) -> &'static str {
        "DEX"
    }
}

pub(crate) struct Dey;
impl OpCode for Dey {
    fn next_func(
        &self,
        _address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(
            |register| register.get_y(),
            |register, data| register.set_y(data),
            decrement,
        ))
    }
    fn name(&self) -> &'static str {
        "DEY"
    }
}

pub(crate) struct Dec;
impl OpCode for Dec {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, decrement))
    }
    fn name(&self) -> &'static str {
        "DEC"
    }
}
