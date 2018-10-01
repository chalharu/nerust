// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Isc;
impl OpCode for Isc {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, |r, v| {
            let result = v.wrapping_add(1);
            let a = u16::from(r.get_a());
            let b = u16::from(result);
            let c = if r.get_c() { 0 } else { 1 };
            let d = a.wrapping_sub(b).wrapping_sub(c);
            let result2 = (d & 0xFF) as u8;
            r.set_nz_from_value(result2);
            r.set_a(result2);
            r.set_c(d <= 0xFF);
            r.set_v((a ^ b) & 0x80 != 0 && (a ^ d) & 0x80 != 0);
            result
        }))
    }
    fn name(&self) -> &'static str {
        "ISC"
    }
}

pub(crate) struct Dcp;
impl OpCode for Dcp {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, |r, v| {
            let data = v.wrapping_sub(1);
            let a = r.get_a();
            r.set_nz_from_value(a.wrapping_sub(data));
            r.set_c(a >= data);
            data
        }))
    }
    fn name(&self) -> &'static str {
        "DCP"
    }
}

pub(crate) struct Slo;
impl OpCode for Slo {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, |r, v| {
            r.set_c(v & 0x80 == 0x80);
            let a = r.get_a();
            let result = v << 1;
            r.set_a(a | result);
            r.set_nz_from_value(a | result);
            result
        }))
    }
    fn name(&self) -> &'static str {
        "SLO"
    }
}

pub(crate) struct Rla;
impl OpCode for Rla {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, |r, v| {
            let c = if r.get_c() { 1 } else { 0 };
            r.set_c(v & 0x80 != 0);
            let wd = (v << 1) | c;
            let value = wd & r.get_a();
            r.set_a(value);
            r.set_nz_from_value(value);
            wd
        }))
    }
    fn name(&self) -> &'static str {
        "RLA"
    }
}

pub(crate) struct Sre;
impl OpCode for Sre {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, |r, v| {
            r.set_c(v & 0x01 != 0);
            let data = v >> 1;
            let value = r.get_a() ^ data;
            r.set_a(value);
            r.set_nz_from_value(value);
            data
        }))
    }
    fn name(&self) -> &'static str {
        "SRE"
    }
}

pub(crate) struct Rra;
impl OpCode for Rra {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(MemStep1::new(address, |r, v| {
            let value = v >> 1 | if r.get_c() { 0x80 } else { 0 };

            let a = usize::from(r.get_a());
            let b = usize::from(value);
            let c = usize::from(v & 0x01);
            let d = a + b + c;
            let result = (d & 0xFF) as u8;
            r.set_nz_from_value(result);
            r.set_a(result);
            r.set_c(d > 0xFF);
            r.set_v((a ^ b) & 0x80 == 0 && (a ^ d) & 0x80 != 0);
            value
        }))
    }
    fn name(&self) -> &'static str {
        "RRA"
    }
}
