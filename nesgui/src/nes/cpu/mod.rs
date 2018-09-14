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
use self::interrupt::{Interrupt, InterruptStatus, IrqReason};
use self::memory::{Memory, MemoryState};
use self::opcodes::*;
use self::register::Register;
use super::*;
use std::{mem, ops};

fn page_crossed<T: ops::Shr<usize>>(a: T, b: T) -> bool
where
    T::Output: PartialEq,
{
    a >> 8 != b >> 8
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct State {
    register: Register,
    interrupt: Interrupt,
    stall: usize,
    mem_state: Option<MemoryState>,
    cycles: u64,
}

impl State {
    pub fn new() -> Self {
        Self {
            register: Register::new(),
            interrupt: Interrupt::new(),
            stall: 0,
            mem_state: Some(MemoryState::new()),
            cycles: 0,
        }
    }

    pub fn trigger_nmi(&mut self) {
        self.interrupt.set_nmi();
    }

    pub fn trigger_irq(&mut self, reason: IrqReason) {
        self.interrupt.set_irq(reason);
    }

    pub fn acknowledge_irq(&mut self, reason: IrqReason) {
        self.interrupt.acknowledge_irq(reason);
    }

    pub fn enable_irq(&mut self, reason: IrqReason) {
        self.interrupt.enable_irq(reason);
    }

    pub fn disable_irq(&mut self, reason: IrqReason) {
        self.interrupt.disable_irq(reason);
    }

    pub fn get_irq_with_reason(&mut self, reason: IrqReason) -> bool {
        self.interrupt.get_irq_with_reason(reason)
    }

    pub fn register(&mut self) -> &mut Register {
        &mut self.register
    }

    pub fn stall_addition(&mut self, value: usize) {
        self.stall += value;
    }

    pub fn step<C: Controller>(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut C,
        apu: &mut Apu,
        wram: &mut [u8; 2048],
        opcode_tables: &[Box<OpCode>; 256],
        addressing_tables: &[Box<AddressingMode>; 256],
    ) {
        self.cycles = self.cycles.wrapping_add(1);
        if self.stall != 0 {
            self.stall -= 1;
            return;
        }

        let mut mem_state = mem::replace(&mut self.mem_state, None);
        {
            let mut memory = Memory::new(
                wram,
                ppu,
                apu,
                controller,
                cartridge,
                mem_state.as_mut().unwrap(),
            );

            let stall = match (
                self.interrupt.reset,
                self.interrupt.started,
                self.interrupt.nmi,
            ) {
                (true, _, _) => {
                    let pc = memory.read_u16(0xFFFC, self);
                    self.interrupt.unset_reset();
                    self.register().set_pc(pc);
                    self.register().set_sp(0xFD);
                    self.register().set_p(0x24);
                    7
                }
                (false, InterruptStatus::Executing, true) => {
                    self.interrupt.nmi = false;
                    InterruptBody.execute(self, &mut memory, 0xFFFA)
                }
                (false, InterruptStatus::Executing, false) => {
                    InterruptBody.execute(self, &mut memory, 0xFFFE)
                }
                (false, InterruptStatus::Detected, true) => Nmi.execute(self, &mut memory, 0),
                (false, InterruptStatus::Detected, false) => Irq.execute(self, &mut memory, 0),
                (false, InterruptStatus::Polling, _) => {
                    // 割り込み検出
                    if self.interrupt.nmi {
                        self.interrupt.started = InterruptStatus::Detected;
                    } else if self.interrupt.get_irq() && !self.register().get_i() {
                        // self.interrupt.get_irq = false;
                        self.interrupt.use_irq();
                        self.interrupt.started = InterruptStatus::Detected;
                    }

                    let pc = self.register().get_pc();
                    let code = memory.read(pc as usize, self) as usize;
                    let addressing = addressing_tables[code].execute(self, &mut memory);
                    // info!(
                    //     "CPU Oprand: {} {}",
                    //     opcode_tables[code].name(),
                    //     addressing_tables[code].name(),
                    // );
                    self.register()
                        .set_pc(pc.wrapping_add(addressing_tables[code].opcode_length()));
                    let cycles = opcode_tables[code].execute(self, &mut memory, addressing.address);

                    addressing.cycles + cycles
                }
            };
            self.stall += stall - 1;
        }

        self.mem_state = mem_state;
    }
}

pub(crate) struct Core {
    opcode_tables: [Box<OpCode>; 256],
    addressing_tables: [Box<AddressingMode>; 256],
    pub(crate) state: State,
}

impl Core {
    pub fn step<C: Controller>(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut C,
        apu: &mut Apu,
        wram: &mut [u8; 2048],
    ) {
        self.state.step(
            ppu,
            cartridge,
            controller,
            apu,
            wram,
            &self.opcode_tables,
            &self.addressing_tables,
        )
    }

    pub fn trigger_nmi(&mut self) {
        self.state.trigger_nmi();
    }

    pub fn new() -> Self {
        Self {
            opcode_tables: [
                // 0x00
                Box::new(Brk),
                Box::new(Ora),
                Box::new(Kil),
                Box::new(Slo),
                Box::new(Nop),
                Box::new(Ora),
                Box::new(AslMem),
                Box::new(Slo),
                // 0x08
                Box::new(Php),
                Box::new(Ora),
                Box::new(AslAcc),
                Box::new(Anc),
                Box::new(Nop),
                Box::new(Ora),
                Box::new(AslMem),
                Box::new(Slo),
                // 0x10
                Box::new(Bpl),
                Box::new(Ora),
                Box::new(Kil),
                Box::new(Slo),
                Box::new(Nop),
                Box::new(Ora),
                Box::new(AslMem),
                Box::new(Slo),
                // 0x18
                Box::new(Clc),
                Box::new(Ora),
                Box::new(Nop),
                Box::new(Slo),
                Box::new(Nop),
                Box::new(Ora),
                Box::new(AslMem),
                Box::new(Slo),
                // 0x20
                Box::new(Jsr),
                Box::new(And),
                Box::new(Kil),
                Box::new(Rla),
                Box::new(Bit),
                Box::new(And),
                Box::new(RolMem),
                Box::new(Rla),
                // 0x28
                Box::new(Plp),
                Box::new(And),
                Box::new(RolAcc),
                Box::new(Anc),
                Box::new(Bit),
                Box::new(And),
                Box::new(RolMem),
                Box::new(Rla),
                // 0x30
                Box::new(Bmi),
                Box::new(And),
                Box::new(Kil),
                Box::new(Rla),
                Box::new(Nop),
                Box::new(And),
                Box::new(RolMem),
                Box::new(Rla),
                // 0x38
                Box::new(Sec),
                Box::new(And),
                Box::new(Nop),
                Box::new(Rla),
                Box::new(Nop),
                Box::new(And),
                Box::new(RolMem),
                Box::new(Rla),
                // 0x40
                Box::new(Rti),
                Box::new(Eor),
                Box::new(Kil),
                Box::new(Sre),
                Box::new(Nop),
                Box::new(Eor),
                Box::new(LsrMem),
                Box::new(Sre),
                // 0x48
                Box::new(Pha),
                Box::new(Eor),
                Box::new(LsrAcc),
                Box::new(Alr),
                Box::new(Jmp),
                Box::new(Eor),
                Box::new(LsrMem),
                Box::new(Sre),
                // 0x50
                Box::new(Bvc),
                Box::new(Eor),
                Box::new(Kil),
                Box::new(Sre),
                Box::new(Nop),
                Box::new(Eor),
                Box::new(LsrMem),
                Box::new(Sre),
                // 0x58
                Box::new(Cli),
                Box::new(Eor),
                Box::new(Nop),
                Box::new(Sre),
                Box::new(Nop),
                Box::new(Eor),
                Box::new(LsrMem),
                Box::new(Sre),
                // 0x60
                Box::new(Rts),
                Box::new(Adc),
                Box::new(Kil),
                Box::new(Rra),
                Box::new(Nop),
                Box::new(Adc),
                Box::new(RorMem),
                Box::new(Rra),
                // 0x68
                Box::new(Pla),
                Box::new(Adc),
                Box::new(RorAcc),
                Box::new(Arr),
                Box::new(Jmp),
                Box::new(Adc),
                Box::new(RorMem),
                Box::new(Rra),
                // 0x70
                Box::new(Bvs),
                Box::new(Adc),
                Box::new(Kil),
                Box::new(Rra),
                Box::new(Nop),
                Box::new(Adc),
                Box::new(RorMem),
                Box::new(Rra),
                // 0x78
                Box::new(Sei),
                Box::new(Adc),
                Box::new(Nop),
                Box::new(Rra),
                Box::new(Nop),
                Box::new(Adc),
                Box::new(RorMem),
                Box::new(Rra),
                // 0x80
                Box::new(Nop),
                Box::new(Sta),
                Box::new(Nop),
                Box::new(Sax),
                Box::new(Sty),
                Box::new(Sta),
                Box::new(Stx),
                Box::new(Sax),
                // 0x88
                Box::new(Dey),
                Box::new(Nop),
                Box::new(Txa),
                Box::new(Xaa),
                Box::new(Sty),
                Box::new(Sta),
                Box::new(Stx),
                Box::new(Sax),
                // 0x90
                Box::new(Bcc),
                Box::new(Sta),
                Box::new(Kil),
                Box::new(Ahx),
                Box::new(Sty),
                Box::new(Sta),
                Box::new(Stx),
                Box::new(Sax),
                // 0x98
                Box::new(Tya),
                Box::new(Sta),
                Box::new(Txs),
                Box::new(Tas),
                Box::new(Shy),
                Box::new(Sta),
                Box::new(Shx),
                Box::new(Ahx),
                // 0xA0
                Box::new(Ldy),
                Box::new(Lda),
                Box::new(Ldx),
                Box::new(Lax),
                Box::new(Ldy),
                Box::new(Lda),
                Box::new(Ldx),
                Box::new(Lax),
                // 0xA8
                Box::new(Tay),
                Box::new(Lda),
                Box::new(Tax),
                Box::new(Lax),
                Box::new(Ldy),
                Box::new(Lda),
                Box::new(Ldx),
                Box::new(Lax),
                // 0xB0
                Box::new(Bcs),
                Box::new(Lda),
                Box::new(Kil),
                Box::new(Lax),
                Box::new(Ldy),
                Box::new(Lda),
                Box::new(Ldx),
                Box::new(Lax),
                // 0xB8
                Box::new(Clv),
                Box::new(Lda),
                Box::new(Tsx),
                Box::new(Las),
                Box::new(Ldy),
                Box::new(Lda),
                Box::new(Ldx),
                Box::new(Lax),
                // 0xC0
                Box::new(Cpy),
                Box::new(Cmp),
                Box::new(Nop),
                Box::new(Dcp),
                Box::new(Cpy),
                Box::new(Cmp),
                Box::new(Dec),
                Box::new(Dcp),
                // 0xC8
                Box::new(Iny),
                Box::new(Cmp),
                Box::new(Dex),
                Box::new(Axs),
                Box::new(Cpy),
                Box::new(Cmp),
                Box::new(Dec),
                Box::new(Dcp),
                // 0xD0
                Box::new(Bne),
                Box::new(Cmp),
                Box::new(Kil),
                Box::new(Dcp),
                Box::new(Nop),
                Box::new(Cmp),
                Box::new(Dec),
                Box::new(Dcp),
                // 0xD8
                Box::new(Cld),
                Box::new(Cmp),
                Box::new(Nop),
                Box::new(Dcp),
                Box::new(Nop),
                Box::new(Cmp),
                Box::new(Dec),
                Box::new(Dcp),
                // 0xE0
                Box::new(Cpx),
                Box::new(Sbc),
                Box::new(Nop),
                Box::new(Isc),
                Box::new(Cpx),
                Box::new(Sbc),
                Box::new(Inc),
                Box::new(Isc),
                // 0xE8
                Box::new(Inx),
                Box::new(Sbc),
                Box::new(Nop),
                Box::new(Sbc),
                Box::new(Cpx),
                Box::new(Sbc),
                Box::new(Inc),
                Box::new(Isc),
                // 0xF0
                Box::new(Beq),
                Box::new(Sbc),
                Box::new(Kil),
                Box::new(Isc),
                Box::new(Nop),
                Box::new(Sbc),
                Box::new(Inc),
                Box::new(Isc),
                // 0xF8
                Box::new(Sed),
                Box::new(Sbc),
                Box::new(Nop),
                Box::new(Isc),
                Box::new(Nop),
                Box::new(Sbc),
                Box::new(Inc),
                Box::new(Isc),
            ],
            addressing_tables: [
                // 0x00
                Box::new(Implied),
                Box::new(IndexedIndirect),
                Box::new(Implied),
                Box::new(IndexedIndirect),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                // 0x08
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Accumulator),
                Box::new(Immediate),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                // 0x10
                Box::new(Relative),
                Box::new(IndirectIndexed),
                Box::new(Implied),
                Box::new(IndirectIndexedRMW),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                // 0x18
                Box::new(Implied),
                Box::new(AbsoluteY),
                Box::new(Implied),
                Box::new(AbsoluteYRMW),
                Box::new(AbsoluteX),
                Box::new(AbsoluteX),
                Box::new(AbsoluteXRMW),
                Box::new(AbsoluteXRMW),
                // 0x20
                Box::new(Absolute),
                Box::new(IndexedIndirect),
                Box::new(Implied),
                Box::new(IndexedIndirect),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                // 0x28
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Accumulator),
                Box::new(Immediate),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                // 0x30
                Box::new(Relative),
                Box::new(IndirectIndexed),
                Box::new(Implied),
                Box::new(IndirectIndexedRMW),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                // 0x38
                Box::new(Implied),
                Box::new(AbsoluteY),
                Box::new(Implied),
                Box::new(AbsoluteYRMW),
                Box::new(AbsoluteX),
                Box::new(AbsoluteX),
                Box::new(AbsoluteXRMW),
                Box::new(AbsoluteXRMW),
                // 0x40
                Box::new(Implied),
                Box::new(IndexedIndirect),
                Box::new(Implied),
                Box::new(IndexedIndirect),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                // 0x48
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Accumulator),
                Box::new(Immediate),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                // 0x50
                Box::new(Relative),
                Box::new(IndirectIndexed),
                Box::new(Implied),
                Box::new(IndirectIndexedRMW),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                // 0x58
                Box::new(Implied),
                Box::new(AbsoluteY),
                Box::new(Implied),
                Box::new(AbsoluteYRMW),
                Box::new(AbsoluteX),
                Box::new(AbsoluteX),
                Box::new(AbsoluteXRMW),
                Box::new(AbsoluteXRMW),
                // 0x60
                Box::new(Implied),
                Box::new(IndexedIndirect),
                Box::new(Implied),
                Box::new(IndexedIndirect),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                // 0x68
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Accumulator),
                Box::new(Immediate),
                Box::new(AbsoluteIndirect),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                // 0x70
                Box::new(Relative),
                Box::new(IndirectIndexed),
                Box::new(Implied),
                Box::new(IndirectIndexedRMW),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                // 0x78
                Box::new(Implied),
                Box::new(AbsoluteY),
                Box::new(Implied),
                Box::new(AbsoluteYRMW),
                Box::new(AbsoluteX),
                Box::new(AbsoluteX),
                Box::new(AbsoluteXRMW),
                Box::new(AbsoluteXRMW),
                // 0x80
                Box::new(Immediate),
                Box::new(IndexedIndirect),
                Box::new(Immediate),
                Box::new(IndexedIndirect),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                // 0x88
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                // 0x90
                Box::new(Relative),
                Box::new(IndirectIndexedRMW),
                Box::new(Implied),
                Box::new(IndirectIndexedRMW),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageY),
                Box::new(ZeroPageY),
                // 0x98
                Box::new(Implied),
                Box::new(AbsoluteYRMW),
                Box::new(Implied),
                Box::new(AbsoluteYRMW),
                Box::new(AbsoluteXRMW),
                Box::new(AbsoluteXRMW),
                Box::new(AbsoluteYRMW),
                Box::new(AbsoluteYRMW),
                // 0xA0
                Box::new(Immediate),
                Box::new(IndexedIndirect),
                Box::new(Immediate),
                Box::new(IndexedIndirect),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                // 0xA8
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                // 0xB0
                Box::new(Relative),
                Box::new(IndirectIndexed),
                Box::new(Implied),
                Box::new(IndirectIndexed),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageY),
                Box::new(ZeroPageY),
                // 0xB8
                Box::new(Implied),
                Box::new(AbsoluteY),
                Box::new(Implied),
                Box::new(AbsoluteY),
                Box::new(AbsoluteX),
                Box::new(AbsoluteX),
                Box::new(AbsoluteY),
                Box::new(AbsoluteY),
                // 0xC0
                Box::new(Immediate),
                Box::new(IndexedIndirect),
                Box::new(Immediate),
                Box::new(IndexedIndirect),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                // 0xC8
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                // 0xD0
                Box::new(Relative),
                Box::new(IndirectIndexed),
                Box::new(Implied),
                Box::new(IndirectIndexedRMW),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                // 0xD8
                Box::new(Implied),
                Box::new(AbsoluteY),
                Box::new(Implied),
                Box::new(AbsoluteYRMW),
                Box::new(AbsoluteX),
                Box::new(AbsoluteX),
                Box::new(AbsoluteXRMW),
                Box::new(AbsoluteXRMW),
                // 0xE0
                Box::new(Immediate),
                Box::new(IndexedIndirect),
                Box::new(Immediate),
                Box::new(IndexedIndirect),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                Box::new(ZeroPage),
                // 0xE8
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Implied),
                Box::new(Immediate),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                Box::new(Absolute),
                // 0xF0
                Box::new(Relative),
                Box::new(IndirectIndexed),
                Box::new(Implied),
                Box::new(IndirectIndexedRMW),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                Box::new(ZeroPageX),
                // 0xF8
                Box::new(Implied),
                Box::new(AbsoluteY),
                Box::new(Implied),
                Box::new(AbsoluteYRMW),
                Box::new(AbsoluteX),
                Box::new(AbsoluteX),
                Box::new(AbsoluteXRMW),
                Box::new(AbsoluteXRMW),
            ],
            state: State::new(),
        }
    }
}
