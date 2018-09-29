// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

fn increment(register: &mut Register, data: u8) -> u8 {
    let result = data.wrapping_add(1);
    register.set_nz_from_value(result);
    result
}

pub(crate) struct Inx;
impl OpCode for Inx {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(
            |register| register.get_x(),
            |register, data| register.set_x(data),
            increment,
        ))
    }
    fn name(&self) -> &'static str {
        "INX"
    }
}

pub(crate) struct Iny;
impl OpCode for Iny {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(
            |register| register.get_y(),
            |register, data| register.set_y(data),
            increment,
        ))
    }
    fn name(&self) -> &'static str {
        "INY"
    }
}

pub(crate) struct Inc;
impl OpCode for Inc {
    fn next_func(&self, address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, increment))
    }
    fn name(&self) -> &'static str {
        "INC"
    }
}
