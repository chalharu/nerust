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
use self::interrupt::Interrupt;
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
// pub(crate) struct State {
//     register: Register,
//     interrupt: Interrupt,
//     stall: usize,
//     mem_state: Option<MemoryState>,
//     cycles: u64,
// }

// impl State {
//     pub fn new() -> Self {
//         Self {
//             register: Register::new(),
//             interrupt: Interrupt::new(),
//             stall: 0,
//             mem_state: Some(MemoryState::new()),
//             cycles: 0,
//         }
//     }

//     pub fn trigger_nmi(&mut self) {
//         self.interrupt.set_nmi();
//     }

//     pub fn trigger_irq(&mut self, reason: IrqReason) {
//         self.interrupt.set_irq(reason);
//     }

//     pub fn acknowledge_irq(&mut self, reason: IrqReason) {
//         self.interrupt.acknowledge_irq(reason);
//     }

//     pub fn enable_irq(&mut self, reason: IrqReason) {
//         self.interrupt.enable_irq(reason);
//     }

//     pub fn disable_irq(&mut self, reason: IrqReason) {
//         self.interrupt.disable_irq(reason);
//     }

//     pub fn get_irq_with_reason(&mut self, reason: IrqReason) -> bool {
//         self.interrupt.get_irq_with_reason(reason)
//     }

//     pub fn register(&mut self) -> &mut Register {
//         &mut self.register
//     }

//     pub fn stall_addition(&mut self, value: usize) {
//         self.stall += value;
//     }

//     pub fn reset(&mut self) {
//         self.interrupt.set_reset();
//     }

//     pub fn step<C: Controller>(
//         &mut self,
//         ppu: &mut Ppu,
//         cartridge: &mut Box<Cartridge>,
//         controller: &mut C,
//         apu: &mut Apu,
//         wram: &mut [u8; 2048],
//         opcode_tables: &Opcodes,
//         addressing_tables: &AddressingModeLut,
//     ) {
//         self.cycles = self.cycles.wrapping_add(1);
//         if self.stall != 0 {
//             self.stall -= 1;
//             return;
//         }

//         let mut mem_state = mem::replace(&mut self.mem_state, None);
//         {
//             let mut memory = Memory::new(
//                 wram,
//                 ppu,
//                 apu,
//                 controller,
//                 cartridge,
//                 mem_state.as_mut().unwrap(),
//             );

//             let stall = match (
//                 self.interrupt.reset,
//                 self.interrupt.started,
//                 self.interrupt.nmi,
//             ) {
//                 (true, _, _) => {
//                     let pc = memory.read_u16(0xFFFC, self);
//                     self.interrupt.unset_reset();
//                     self.register().set_pc(pc);
//                     self.register().set_i(true);
//                     let sp = self.register().get_sp().wrapping_sub(3);
//                     self.register().set_sp(sp);
//                     7
//                 }
//                 (false, InterruptStatus::Executing, true) => {
//                     self.interrupt.nmi = false;
//                     InterruptBody.execute(self, &mut memory, 0xFFFA)
//                 }
//                 (false, InterruptStatus::Executing, false) => {
//                     InterruptBody.execute(self, &mut memory, 0xFFFE)
//                 }
//                 (false, InterruptStatus::Detected, true) => Nmi.execute(self, &mut memory, 0),
//                 (false, InterruptStatus::Detected, false) => Irq.execute(self, &mut memory, 0),
//                 (false, InterruptStatus::Polling, _) => {
//                     // 割り込み検出
//                     if self.interrupt.nmi {
//                         self.interrupt.started = InterruptStatus::Detected;
//                     } else if self.interrupt.get_irq() && !self.register().get_i() {
//                         self.interrupt.use_irq();
//                         self.interrupt.started = InterruptStatus::Detected;
//                     }

//                     let pc = self.register().get_pc();
//                     let code = memory.read(pc as usize, self);
//                     let addressing = addressing_tables.get(code).execute(self, &mut memory);
//                     // info!(
//                     //     "CPU Oprand: {} {}",
//                     //     opcode_tables[code].name(),
//                     //     addressing_tables[code].name(),
//                     // );
//                     self.register()
//                         .set_pc(pc.wrapping_add(addressing_tables.get(code).opcode_length()));
//                     let cycles =
//                         opcode_tables
//                             .get(code)
//                             .execute(self, &mut memory, addressing.address);

//                     addressing.cycles + cycles
//                 }
//             };
//             self.stall += stall - 1;
//         }

//         self.mem_state = mem_state;
//     }
// }

// pub(crate) struct Core {
//     opcode_tables: Opcodes,
//     addressing_tables: AddressingModeLut,
//     pub(crate) state: State,
// }

// impl Core {
//     pub fn step<C: Controller>(
//         &mut self,
//         ppu: &mut Ppu,
//         cartridge: &mut Box<Cartridge>,
//         controller: &mut C,
//         apu: &mut Apu,
//         wram: &mut [u8; 2048],
//     ) {
//         self.state.step(
//             ppu,
//             cartridge,
//             controller,
//             apu,
//             wram,
//             &self.opcode_tables,
//             &self.addressing_tables,
//         )
//     }

//     pub fn trigger_nmi(&mut self) {
//         self.state.trigger_nmi();
//     }

//     pub fn reset(&mut self) {
//         self.state.reset();
//     }

//     pub fn new() -> Self {
//         Self {
//             opcode_tables: Opcodes::new(),
//             addressing_tables: AddressingModeLut::new(),
//             state: State::new(),
//         }
//     }
// }

// #[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Core {
    opcode_tables: Opcodes,
    addressing_tables: AddressingModeLut,
    memory: Memory,
    register: Register,
    pub(crate) interrupt: Interrupt,
    cycles: u64,
    next_func: Box<dyn CpuStepState>,
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
        }
    }

    pub fn step(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) {
        self.cycles = self.cycles.wrapping_add(1);

        if !self.interrupt.running_dma {
            self.interrupt.executing = self.interrupt.detected;
            self.interrupt.detected = self.interrupt.nmi
                || ((self.interrupt.irq_flag & self.interrupt.irq_mask) != 0
                    && !self.register.get_i());
        }

        // 身代わりパターン
        self.next_func = (::std::mem::replace(&mut self.next_func, Box::new(Dummy)))
            .next(self, ppu, cartridge, controller, apu);

        // let addressing = addressing_tables.get(code).execute(self, &mut memory);
        // self.register()
        //     .set_pc(pc.wrapping_add(addressing_tables.get(code).opcode_length()));
        // let cycles = opcode_tables
        //     .get(code)
        //     .execute(self, &mut memory, addressing.address);
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
