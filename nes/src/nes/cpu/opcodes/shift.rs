// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

fn asl(register: &mut Register, data: u8) -> u8 {
    register.set_c(data & 0x80 != 0);
    let value = data << 1;
    register.set_nz_from_value(value);
    value
}

accumulate!(AslAcc, Register::get_a, Register::set_a, asl);
accumulate_memory!(AslMem, asl);

fn lsr(register: &mut Register, data: u8) -> u8 {
    register.set_c(data & 0x01 != 0);
    let value = data >> 1;
    register.set_nz_from_value(value);
    value
}

accumulate!(LsrAcc, Register::get_a, Register::set_a, lsr);
accumulate_memory!(LsrMem, lsr);

fn rol(register: &mut Register, data: u8) -> u8 {
    let c = if register.get_c() { 1 } else { 0 };
    register.set_c(data & 0x80 != 0);
    let value = data << 1 | c;
    register.set_nz_from_value(value);
    value
}

accumulate!(RolAcc, Register::get_a, Register::set_a, rol);
accumulate_memory!(RolMem, rol);

fn ror(register: &mut Register, data: u8) -> u8 {
    let c = if register.get_c() { 0x80 } else { 0 };
    register.set_c(data & 0x01 != 0);
    let value = data >> 1 | c;
    register.set_nz_from_value(value);
    value
}

accumulate!(RorAcc, Register::get_a, Register::set_a, ror);
accumulate_memory!(RorMem, ror);
