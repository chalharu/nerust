// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod addressing_mode;
pub mod interrupt;
mod memory;
mod opcodes;
mod register;

use self::addressing_mode::*;
use self::interrupt::{Interrupt, IrqSource};
use self::memory::Memory;
use self::opcodes::{
    interrupt::{Irq, Reset},
    *,
};
use self::register::{Register, RegisterP};
use super::*;
use std::ops::Shr;

fn page_crossed<T: Shr<usize>>(a: T, b: T) -> bool
where
    T::Output: PartialEq,
{
    a >> 8 != b >> 8
}

const NMI_VECTOR: usize = 0xFFFA;
const RESET_VECTOR: usize = 0xFFFC;
const IRQ_VECTOR: usize = 0xFFFE;

// #[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Core {
    opcode_tables: Opcodes,
    addressing_tables: AddressingModeLut,
    memory: Memory,
    register: Register,
    pub(crate) interrupt: Interrupt,
    cycles: u64,
    next_func: Box<dyn CpuStepState>,
    oam_dma: Option<Box<dyn OamDmaStepState>>,
}

impl Core {
    pub fn new() -> Self {
        Self {
            opcode_tables: Opcodes::new(),
            addressing_tables: AddressingModeLut::new(),
            register: Register::new(),
            interrupt: Interrupt::new(),
            memory: Memory::new(),
            cycles: 0,
            next_func: Reset::new(),
            oam_dma: None,
        }
    }

    pub fn reset(&mut self) {
        self.interrupt.reset();
        self.oam_dma = None;
        self.next_func = Reset::new();
        self.cycles = 0;
    }

    pub fn step(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) {
        self.cycles = self.cycles.wrapping_add(1);

        if self.interrupt.dmc_start {
            self.interrupt.dmc_start = false;
            self.interrupt.dmc_count = if let Some(oam_dma) = &self.oam_dma {
                match oam_dma.count() {
                    0 => 3,
                    1 => 1,
                    _ => 2,
                }
            } else {
                4
            };
        }

        if self.interrupt.dmc_count > 0 && (self.cycles & 1 == 0) {
            self.interrupt.dmc_count -= 1;
            if self.interrupt.dmc_count == 0 {
                if let Some(addr) = apu.dmc_fill_address() {
                    let value = self.memory.read(
                        addr,
                        ppu,
                        cartridge,
                        controller,
                        apu,
                        &mut self.interrupt,
                    );
                    apu.dmc_fill(value, &mut self.interrupt);
                }
            }
        } else {
            if let Some(offset) = ::std::mem::replace(&mut self.interrupt.oam_dma, None) {
                self.oam_dma = Some(OamDma::new(offset));
            }

            if let Some(mut oam_dma) = ::std::mem::replace(&mut self.oam_dma, None) {
                self.oam_dma = oam_dma.next(self, ppu, cartridge, controller, apu);
            } else {
                self.interrupt.executing = self.interrupt.detected;
                // 身代わりパターン
                self.next_func = (::std::mem::replace(&mut self.next_func, Box::new(Dummy)))
                    .next(self, ppu, cartridge, controller, apu);
                self.interrupt.detected = self.interrupt.nmi
                    || (!((self.interrupt.irq_flag & self.interrupt.irq_mask).is_empty())
                        && !self.register.get_i());

            }
        }
    }
}

struct Dummy;
impl CpuStepState for Dummy {
    fn next(
        &mut self,
        _core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Box<Cartridge>,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        Box::new(Dummy)
    }
}

pub(crate) trait CpuStepState {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState>;
}

struct FetchOpCode {}

impl FetchOpCode {
    pub fn new(interrupt: &Interrupt) -> Box<dyn CpuStepState> {
        if interrupt.executing {
            Box::new(Irq::new())
        } else {
            Box::new(Self {})
        }
    }
}

impl CpuStepState for FetchOpCode {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let code = usize::from(core.memory.read_next(
            &mut core.register,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        ));
        core.addressing_tables.get(code).next_func(
            code,
            &mut core.register,
            &mut core.opcode_tables,
            &mut core.interrupt,
        )
    }
}

pub(crate) trait OamDmaStepState {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Option<Box<dyn OamDmaStepState>>;

    fn count(&self) -> u8;
}

struct OamDma {
    offset: u8,
}

impl OamDma {
    pub fn new(offset: u8) -> Box<dyn OamDmaStepState> {
        Box::new(Self { offset })
    }
}

impl OamDmaStepState for OamDma {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Option<Box<dyn OamDmaStepState>> {
        // dummy read
        read_dummy_current(core, ppu, cartridge, controller, apu);
        if core.cycles & 1 != 0 {
            Some(OamDma::new(self.offset))
        } else {
            Some(OamDmaStep1::new(self.offset, 255))
        }
    }

    fn count(&self) -> u8 {
        255
    }
}

struct OamDmaStep1 {
    offset: u8,
    count: u8,
}

impl OamDmaStep1 {
    pub fn new(offset: u8, count: u8) -> Box<dyn OamDmaStepState> {
        Box::new(Self { offset, count })
    }
}

impl OamDmaStepState for OamDmaStep1 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Option<Box<dyn OamDmaStepState>> {
        let value = core.memory.read(
            usize::from(self.offset) * 0x100 + usize::from(255 - self.count),
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        Some(OamDmaStep2::new(self.offset, self.count, value))
    }

    fn count(&self) -> u8 {
        self.count
    }
}

struct OamDmaStep2 {
    offset: u8,
    count: u8,
    value: u8,
}

impl OamDmaStep2 {
    pub fn new(offset: u8, count: u8, value: u8) -> Box<dyn OamDmaStepState> {
        Box::new(Self {
            offset,
            count,
            value,
        })
    }
}

impl OamDmaStepState for OamDmaStep2 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Option<Box<dyn OamDmaStepState>> {
        core.memory.write(
            0x2004,
            self.value,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        if self.count == 0 {
            None
        } else {
            Some(OamDmaStep1::new(self.offset, self.count - 1))
        }
    }

    fn count(&self) -> u8 {
        self.count
    }
}

fn push(
    core: &mut Core,
    ppu: &mut Ppu,
    cartridge: &mut Box<Cartridge>,
    controller: &mut Controller,
    apu: &mut Apu,
    value: u8,
) {
    let sp = core.register.get_sp();
    core.register.set_sp(sp.wrapping_sub(1));
    core.memory.write(
        0x100 | usize::from(sp),
        value,
        ppu,
        cartridge,
        controller,
        apu,
        &mut core.interrupt,
    );
}

fn pull(
    core: &mut Core,
    ppu: &mut Ppu,
    cartridge: &mut Box<Cartridge>,
    controller: &mut Controller,
    apu: &mut Apu,
) -> u8 {
    let sp = core.register.get_sp().wrapping_add(1);
    core.register.set_sp(sp);
    core.memory.read(
        0x100 | usize::from(sp),
        ppu,
        cartridge,
        controller,
        apu,
        &mut core.interrupt,
    )
}

fn read_dummy_current(
    core: &mut Core,
    ppu: &mut Ppu,
    cartridge: &mut Box<Cartridge>,
    controller: &mut Controller,
    apu: &mut Apu,
) {
    let pc = usize::from(core.register.get_pc());
    let _ = core
        .memory
        .read(pc, ppu, cartridge, controller, apu, &mut core.interrupt);
}
