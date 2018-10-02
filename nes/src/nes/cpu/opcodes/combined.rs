// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Lax;
impl OpCode for Lax {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(ReadStep1::new(address, |r, v| {
            r.set_a(v);
            r.set_x(v);
            r.set_nz_from_value(v);
        }))
    }
    fn name(&self) -> &'static str {
        "LAX"
    }
}

pub(crate) struct Anc;
impl OpCode for Anc {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(ReadStep1::new(address, |r, v| {
            let result = r.get_a() & v;
            r.set_nz_from_value(result);
            r.set_c(result & 0x80 != 0);
            r.set_a(result);
        }))
    }
    fn name(&self) -> &'static str {
        "ANC"
    }
}

pub(crate) struct Alr;
impl OpCode for Alr {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(ReadStep1::new(address, |r, v| {
            let result = r.get_a() & v;
            r.set_c(result & 0x01 != 0);
            let value = result >> 1;
            r.set_nz_from_value(value);
            r.set_a(value);
        }))
    }
    fn name(&self) -> &'static str {
        "ALR"
    }
}

pub(crate) struct Arr;
impl OpCode for Arr {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(ReadStep1::new(address, |r, v| {
            let result = r.get_a() & v;
            let value = result >> 1 | if r.get_c() { 0x80 } else { 0 };
            r.set_c(result & 0x80 != 0);
            r.set_v((((value >> 6) & 1) != 0) ^ (((value >> 5) & 1) != 0));
            r.set_nz_from_value(value);
            r.set_a(value);
        }))
    }
    fn name(&self) -> &'static str {
        "ARR"
    }
}

pub(crate) struct Xaa;
impl OpCode for Xaa {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(ReadStep1::new(address, |r, v| {
            let result = r.get_x() & v;
            r.set_nz_from_value(result);
            r.set_a(result);
        }))
    }
    fn name(&self) -> &'static str {
        "XAA"
    }
}

pub(crate) struct Las;
impl OpCode for Las {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(ReadStep1::new(address, |r, v| {
            let result = r.get_sp() & v;
            r.set_nz_from_value(result);
            r.set_a(result);
            r.set_x(result);
            r.set_sp(result);
        }))
    }
    fn name(&self) -> &'static str {
        "LAS"
    }
}

pub(crate) struct Axs;
impl OpCode for Axs {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(ReadStep1::new(address, |r, v| {
            let a = u16::from(r.get_a() & r.get_x());
            let b = u16::from(v);
            let d = a.wrapping_sub(b);

            let result = (d & 0xFF) as u8;
            r.set_nz_from_value(result);
            r.set_x(result);
            r.set_c(d <= 0xFF);
        }))
    }
    fn name(&self) -> &'static str {
        "AXS"
    }
}

pub(crate) struct Sax;
impl OpCode for Sax {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(WriteStep1::new(address, |r| r.get_a() & r.get_x()))
    }
    fn name(&self) -> &'static str {
        "SAX"
    }
}

pub(crate) struct Ahx;
impl OpCode for Ahx {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        let high = ((address >> 8) as u8).wrapping_add(1);
        Box::new(WriteStep1::new(address, move |r| {
            r.get_a() & r.get_x() & high
        }))
    }
    fn name(&self) -> &'static str {
        "AHX"
    }
}

pub(crate) struct Tas;
impl OpCode for Tas {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        Box::new(WriteStep1::new(address, |r| {
            let sp = r.get_a() & r.get_x();
            r.set_sp(sp);
            sp & ((r.get_pc() >> 8) as u8).wrapping_add(1)
        }))
    }
    fn name(&self) -> &'static str {
        "TAS"
    }
}

pub(crate) struct Shx;
impl OpCode for Shx {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        let high = ((address >> 8) as u8).wrapping_add(1);
        let low = address & 0xFF;
        Box::new(WriteNewAddrStep1::new(address, move |r| {
            let value = r.get_x() & high;
            let new_addr = (usize::from(value) << 8) | low;
            (value, new_addr)
        }))
    }
    fn name(&self) -> &'static str {
        "SHX"
    }
}

pub(crate) struct Shy;
impl OpCode for Shy {
    fn next_func(
        &self,
        address: usize,
        _register: &mut Register,
        _interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        let high = ((address >> 8) as u8).wrapping_add(1);
        let low = address & 0xFF;
        Box::new(WriteNewAddrStep1::new(address, move |r| {
            let value = r.get_y() & high;
            let new_addr = (usize::from(value) << 8) | low;
            (value, new_addr)
        }))
    }
    fn name(&self) -> &'static str {
        "SHY"
    }
}

struct ReadStep1<F: Fn(&mut Register, u8) -> ()> {
    address: usize,
    func: F,
}

impl<F: Fn(&mut Register, u8) -> ()> ReadStep1<F> {
    pub fn new(address: usize, func: F) -> Self {
        Self { address, func }
    }
}

impl<F: Fn(&mut Register, u8) -> ()> CpuStepState for ReadStep1<F> {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let data = core.memory.read(
            self.address,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        (self.func)(&mut core.register, data);
        FetchOpCode::new(&core.interrupt)
    }
}

struct WriteStep1<F: Fn(&mut Register) -> u8> {
    address: usize,
    func: F,
}

impl<F: Fn(&mut Register) -> u8> WriteStep1<F> {
    pub fn new(address: usize, func: F) -> Self {
        Self { address, func }
    }
}

impl<F: Fn(&mut Register) -> u8> CpuStepState for WriteStep1<F> {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let data = (self.func)(&mut core.register);
        core.memory.write(
            self.address,
            data,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        FetchOpCode::new(&core.interrupt)
    }
}

struct WriteNewAddrStep1<F: Fn(&mut Register) -> (u8, usize)> {
    address: usize,
    func: F,
}

impl<F: Fn(&mut Register) -> (u8, usize)> WriteNewAddrStep1<F> {
    pub fn new(address: usize, func: F) -> Self {
        Self { address, func }
    }
}

impl<F: Fn(&mut Register) -> (u8, usize)> CpuStepState for WriteNewAddrStep1<F> {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let (data, address) = (self.func)(&mut core.register);
        core.memory.write(
            address,
            data,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        FetchOpCode::new(&core.interrupt)
    }
}
