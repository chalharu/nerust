// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

fn get_reg_a(register: &mut Register) -> u8 {
    register.get_a()
}

fn set_reg_a(register: &mut Register, data: u8) {
    register.set_a(data);
}

fn asl(register: &mut Register, data: u8) -> u8 {
    register.set_c(data & 0x80 != 0);
    let value = data << 1;
    register.set_nz_from_value(value);
    value
}

pub(crate) struct AslAcc;
impl OpCode for AslAcc {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(get_reg_a, set_reg_a, asl))
    }
    fn name(&self) -> &'static str {
        "ASL"
    }
}

pub(crate) struct AslMem;
impl OpCode for AslMem {
    fn next_func(&self, address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, asl))
    }
    fn name(&self) -> &'static str {
        "ASL"
    }
}

fn lsr(register: &mut Register, data: u8) -> u8 {
    register.set_c(data & 0x01 != 0);
    let value = data >> 1;
    register.set_nz_from_value(value);
    value
}

pub(crate) struct LsrAcc;
impl OpCode for LsrAcc {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(get_reg_a, set_reg_a, lsr))
    }
    fn name(&self) -> &'static str {
        "LSR"
    }
}

pub(crate) struct LsrMem;
impl OpCode for LsrMem {
    fn next_func(&self, address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, lsr))
    }
    fn name(&self) -> &'static str {
        "LSR"
    }
}

fn rol(register: &mut Register, data: u8) -> u8 {
    let c = if register.get_c() { 1 } else { 0 };
    register.set_c(data & 0x80 != 0);
    let value = data << 1 | c;
    register.set_nz_from_value(value);
    value
}

pub(crate) struct RolAcc;
impl OpCode for RolAcc {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(get_reg_a, set_reg_a, rol))
    }
    fn name(&self) -> &'static str {
        "ROL"
    }
}

pub(crate) struct RolMem;
impl OpCode for RolMem {
    fn next_func(&self, address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, rol))
    }
    fn name(&self) -> &'static str {
        "ROL"
    }
}

fn ror(register: &mut Register, data: u8) -> u8 {
    let c = if register.get_c() { 0x80 } else { 0 };
    register.set_c(data & 0x01 != 0);
    let value = data >> 1 | c;
    register.set_nz_from_value(value);
    value
}

pub(crate) struct RorAcc;
impl OpCode for RorAcc {
    fn next_func(&self, _address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(AccStep1::new(get_reg_a, set_reg_a, ror))
    }
    fn name(&self) -> &'static str {
        "ROR"
    }
}

pub(crate) struct RorMem;
impl OpCode for RorMem {
    fn next_func(&self, address: usize, _register: &mut Register) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, ror))
    }
    fn name(&self) -> &'static str {
        "ROR"
    }
}
