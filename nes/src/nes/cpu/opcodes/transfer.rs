// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

fn set_nz_from_value(register: &mut Register, data: u8) -> u8 {
    register.set_nz_from_value(data);
    data
}

pub(crate) struct Tax;
impl OpCode for Tax {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(
            |r| r.get_a(),
            |r, v| r.set_x(v),
            set_nz_from_value,
        ))
    }
    fn name(&self) -> &'static str {
        "TAX"
    }
}

pub(crate) struct Tay;
impl OpCode for Tay {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(
            |r| r.get_a(),
            |r, v| r.set_y(v),
            set_nz_from_value,
        ))
    }
    fn name(&self) -> &'static str {
        "TAY"
    }
}

pub(crate) struct Tsx;
impl OpCode for Tsx {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(
            |r| r.get_sp(),
            |r, v| r.set_x(v),
            set_nz_from_value,
        ))
    }
    fn name(&self) -> &'static str {
        "TSX"
    }
}

pub(crate) struct Txa;
impl OpCode for Txa {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(
            |r| r.get_x(),
            |r, v| r.set_a(v),
            set_nz_from_value,
        ))
    }
    fn name(&self) -> &'static str {
        "TXA"
    }
}

pub(crate) struct Tya;
impl OpCode for Tya {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(
            |r| r.get_y(),
            |r, v| r.set_a(v),
            set_nz_from_value,
        ))
    }
    fn name(&self) -> &'static str {
        "TYA"
    }
}

pub(crate) struct Txs;
impl OpCode for Txs {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(|r| r.get_x(), |r, v| r.set_sp(v), |_, v| v))
    }
    fn name(&self) -> &'static str {
        "TXS"
    }
}
