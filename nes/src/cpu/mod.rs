// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod addressing_mode;
mod interrupt;
mod memory;
mod opcodes;
mod register;

use self::addressing_mode::*;
use self::interrupt::Interrupt;
use self::memory::Memory;
use self::opcodes::*;
use self::register::Register;
use super::*;

fn page_crossed<T: std::ops::Shr<usize>>(a: T, b: T) -> bool
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
}

impl State {
    pub fn new() -> Self {
        Self {
            register: Register::new(),
            interrupt: Interrupt::None,
            stall: 0,
        }
    }

    pub fn trigger_nmi(&mut self) {
        self.interrupt = Interrupt::Nmi;
    }

    pub fn trigger_irq(&mut self) {
        if !self.register.get_i() {
            self.interrupt = Interrupt::Irq;
        }
    }

    pub fn register(&mut self) -> &mut Register {
        &mut self.register
    }

    pub fn stall_addition(&mut self, value: usize) {
        self.stall += value;
    }

    pub fn step(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
        wram: &mut [u8; 2048],
        opcode_tables: &[&OpCode; 256],
        addressing_tables: &[&AddressingMode; 256],
    ) {
        if self.stall != 0 {
            self.stall -= 1;
            return;
        }

        let mut memory = Memory::new(wram, ppu, apu, controller, cartridge);

        let stall = match self.interrupt {
            Interrupt::Nmi => {
                self.interrupt = Interrupt::None;
                Nmi.execute(self, &mut memory, 0)
            }
            Interrupt::Irq => {
                self.interrupt = Interrupt::None;
                Irq.execute(self, &mut memory, 0)
            }
            Interrupt::None => {
                let pc = self.register().get_pc();
                let code = memory.read(pc as usize) as usize;
                let addressing = addressing_tables[code].execute(self, &mut memory);
                let cycles = opcode_tables[code].execute(self, &mut memory, addressing.address);
                addressing.cycles + cycles
            }
        };
        self.stall += stall;
    }
}

pub(crate) struct Core<'a> {
    opcode_tables: [&'a OpCode; 256],
    addressing_tables: [&'a AddressingMode; 256],
    state: State,
}

impl<'a> Core<'a> {
    pub fn step(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
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

    pub fn trigger_irq(&mut self) {
        self.state.trigger_irq();
    }

    pub fn stall_addition(&mut self, value: usize) {
        self.state.stall_addition(value);
    }

    pub fn new() -> Self {
        Self {
            opcode_tables: [
                &Brk, &Ora, &Kil, &Slo, &Nop, &Ora, &AslMem, &Slo, // 0x00
                &Php, &Ora, &AslAcc, &Anc, &Nop, &Ora, &AslMem, &Slo, // 0x08
                &Bpl, &Ora, &Kil, &Slo, &Nop, &Ora, &AslMem, &Slo, // 0x10
                &Clc, &Ora, &Nop, &Slo, &Nop, &Ora, &AslMem, &Slo, // 0x18
                &Jsr, &And, &Kil, &Rla, &Bit, &And, &RolMem, &Rla, // 0x20
                &Plp, &And, &RolAcc, &Anc, &Bit, &And, &RolMem, &Rla, // 0x28
                &Bmi, &And, &Kil, &Rla, &Nop, &And, &RolMem, &Rla, // 0x30
                &Sec, &And, &Nop, &Rla, &Nop, &And, &RolMem, &Rla, // 0x38
                &Rti, &Eor, &Kil, &Sre, &Nop, &Eor, &LsrMem, &Sre, // 0x40
                &Pha, &Eor, &LsrAcc, &Alr, &Jmp, &Eor, &LsrMem, &Sre, // 0x48
                &Bvc, &Eor, &Kil, &Sre, &Nop, &Eor, &LsrMem, &Sre, // 0x50
                &Cli, &Eor, &Nop, &Sre, &Nop, &Eor, &LsrMem, &Sre, // 0x58
                &Rts, &Adc, &Kil, &Rra, &Nop, &Adc, &RorMem, &Rra, // 0x60
                &Pla, &Adc, &RorAcc, &Arr, &Jmp, &Adc, &RorMem, &Rra, // 0x68
                &Bvs, &Adc, &Kil, &Rra, &Nop, &Adc, &RorMem, &Rra, // 0x70
                &Sei, &Adc, &Nop, &Rra, &Nop, &Adc, &RorMem, &Rra, // 0x78
                &Nop, &Sta, &Nop, &Sax, &Sty, &Sta, &Stx, &Sax, // 0x80
                &Dey, &Nop, &Txa, &Xaa, &Sty, &Sta, &Stx, &Sax, // 0x88
                &Bcc, &Sta, &Kil, &Ahx, &Sty, &Sta, &Stx, &Sax, // 0x90
                &Tya, &Sta, &Txs, &Tas, &Shy, &Sta, &Shx, &Ahx, // 0x98
                &Ldy, &Lda, &Ldx, &Lax, &Ldy, &Lda, &Ldx, &Lax, // 0xA0
                &Tay, &Lda, &Tax, &Lax, &Ldy, &Lda, &Ldx, &Lax, // 0xA8
                &Bcs, &Lda, &Kil, &Lax, &Ldy, &Lda, &Ldx, &Lax, // 0xB0
                &Clv, &Lda, &Tsx, &Las, &Ldy, &Lda, &Ldx, &Lax, // 0xB8
                &Cpy, &Cmp, &Nop, &Dcp, &Cpy, &Cmp, &Dec, &Dcp, // 0xC0
                &Iny, &Cmp, &Dex, &Axs, &Cpy, &Cmp, &Dec, &Dcp, // 0xC8
                &Bne, &Cmp, &Kil, &Dcp, &Nop, &Cmp, &Dec, &Dcp, // 0xD0
                &Cld, &Cmp, &Nop, &Dcp, &Nop, &Cmp, &Dec, &Dcp, // 0xD8
                &Cpx, &Sbc, &Nop, &Isb, &Cpx, &Sbc, &Inc, &Isb, // 0xE0
                &Inx, &Sbc, &Nop, &Sbc, &Cpx, &Sbc, &Inc, &Isb, // 0xE8
                &Beq, &Sbc, &Kil, &Isb, &Nop, &Sbc, &Inc, &Isb, // 0xF0
                &Sed, &Sbc, &Nop, &Isb, &Nop, &Sbc, &Inc, &Isb, // 0xF8
            ],
            addressing_tables: [
                &Implied,
                &IndexedIndirect,
                &Implied,
                &IndexedIndirect,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &Implied,
                &Immediate,
                &Accumulator,
                &Immediate,
                &Absolute,
                &Absolute,
                &Absolute,
                &Absolute,
                &Relative,
                &IndirectIndexed,
                &Implied,
                &IndirectIndexed,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &Implied,
                &AbsoluteY,
                &Implied,
                &AbsoluteY,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
                &Absolute,
                &IndexedIndirect,
                &Implied,
                &IndexedIndirect,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &Implied,
                &Immediate,
                &Accumulator,
                &Immediate,
                &Absolute,
                &Absolute,
                &Absolute,
                &Absolute,
                &Relative,
                &IndirectIndexed,
                &Implied,
                &IndirectIndexed,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &Implied,
                &AbsoluteY,
                &Implied,
                &AbsoluteY,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
                &Implied,
                &IndexedIndirect,
                &Implied,
                &IndexedIndirect,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &Implied,
                &Immediate,
                &Accumulator,
                &Immediate,
                &Absolute,
                &Absolute,
                &Absolute,
                &Absolute,
                &Relative,
                &IndirectIndexed,
                &Implied,
                &IndirectIndexed,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &Implied,
                &AbsoluteY,
                &Implied,
                &AbsoluteY,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
                &Implied,
                &IndexedIndirect,
                &Implied,
                &IndexedIndirect,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &Implied,
                &Immediate,
                &Accumulator,
                &Immediate,
                &AbsoluteIndirect,
                &Absolute,
                &Absolute,
                &Absolute,
                &Relative,
                &IndirectIndexed,
                &Implied,
                &IndirectIndexed,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &Implied,
                &AbsoluteY,
                &Implied,
                &AbsoluteY,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
                &Immediate,
                &IndexedIndirect,
                &Immediate,
                &IndexedIndirect,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &Implied,
                &Immediate,
                &Implied,
                &Immediate,
                &Absolute,
                &Absolute,
                &Absolute,
                &Absolute,
                &Relative,
                &IndirectIndexed,
                &Implied,
                &IndirectIndexed,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageY,
                &ZeroPageY,
                &Implied,
                &AbsoluteY,
                &Implied,
                &AbsoluteY,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteY,
                &AbsoluteY,
                &Immediate,
                &IndexedIndirect,
                &Immediate,
                &IndexedIndirect,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &Implied,
                &Immediate,
                &Implied,
                &Immediate,
                &Absolute,
                &Absolute,
                &Absolute,
                &Absolute,
                &Relative,
                &IndirectIndexed,
                &Implied,
                &IndirectIndexed,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageY,
                &ZeroPageY,
                &Implied,
                &AbsoluteY,
                &Implied,
                &AbsoluteY,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteY,
                &AbsoluteY,
                &Immediate,
                &IndexedIndirect,
                &Immediate,
                &IndexedIndirect,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &Implied,
                &Immediate,
                &Implied,
                &Immediate,
                &Absolute,
                &Absolute,
                &Absolute,
                &Absolute,
                &Relative,
                &IndirectIndexed,
                &Implied,
                &IndirectIndexed,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &Implied,
                &AbsoluteY,
                &Implied,
                &AbsoluteY,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
                &Immediate,
                &IndexedIndirect,
                &Immediate,
                &IndexedIndirect,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &ZeroPage,
                &Implied,
                &Immediate,
                &Implied,
                &Immediate,
                &Absolute,
                &Absolute,
                &Absolute,
                &Absolute,
                &Relative,
                &IndirectIndexed,
                &Implied,
                &IndirectIndexed,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &ZeroPageX,
                &Implied,
                &AbsoluteY,
                &Implied,
                &AbsoluteY,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
                &AbsoluteX,
            ],
            state: State::new(),
        }
    }
}
