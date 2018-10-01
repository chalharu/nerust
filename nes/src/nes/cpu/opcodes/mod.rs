// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod arithmetic;
mod bit;
mod combined;
mod compare;
mod condition_jump;
mod decrement;
mod flag_control;
mod increment;
pub mod interrupt;
mod jump;
mod load;
mod nop;
mod rmw;
mod shift;
mod stack;
mod store;
mod transfer;

use self::arithmetic::*;
use self::bit::*;
use self::combined::*;
use self::compare::*;
use self::condition_jump::*;
use self::decrement::*;
use self::flag_control::*;
use self::increment::*;
use self::interrupt::*;
use self::jump::*;
use self::load::*;
use self::nop::*;
use self::rmw::*;
use self::shift::*;
use self::stack::*;
use self::store::*;
use self::transfer::*;
use super::*;

// fn push(state: &mut State, memory: &mut Memory, value: u8) {
//     let sp = state.register().get_sp();
//     state.stall += memory.write(0x100 | usize::from(sp), value, state);
//     state.register().set_sp(sp.wrapping_sub(1));
// }

// fn pull(state: &mut State, memory: &mut Memory) -> u8 {
//     let sp = state.register().get_sp().wrapping_add(1);
//     state.register().set_sp(sp);
//     memory.read(usize::from(sp) | 0x100, state)
// }

// fn push_u16(state: &mut State, memory: &mut Memory, value: u16) {
//     let hi = (value >> 8) as u8;
//     let low = (value & 0xFF) as u8;
//     push(state, memory, hi);
//     push(state, memory, low);
// }

// fn pull_u16(state: &mut State, memory: &mut Memory) -> u16 {
//     let low = u16::from(pull(state, memory));
//     let hi = u16::from(pull(state, memory));
//     (hi << 8) | low
// }

pub(crate) trait OpCode {
    fn next_func(
        &self,
        address: usize,
        register: &mut Register,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState>;
    fn name(&self) -> &'static str;
}

// pub(crate) struct Nmi;
// impl OpCode for Nmi {
//     fn execute(&self, state: &mut State, memory: &mut Memory, _address: usize) -> usize {
//         let pc = state.register().get_pc();
//         let _ = memory.read(pc as usize, state); // dummy fetch
//         state.register().set_b(false);
//         push_u16(state, memory, pc);
//         let data = state.register().get_p();
//         push(state, memory, data);
//         state.register().set_i(true);
//         state.interrupt.started = InterruptStatus::Executing;
//         4
//     }
//     fn name(&self) -> &'static str {
//         "NMI"
//     }
// }

// pub(crate) struct Irq;
// impl OpCode for Irq {
//     fn execute(&self, state: &mut State, memory: &mut Memory, _address: usize) -> usize {
//         let pc = state.register().get_pc();
//         let _ = memory.read(pc as usize, state); // dummy fetch
//         state.register().set_b(false);

//         push_u16(state, memory, pc);
//         let data = state.register().get_p();
//         push(state, memory, data);

//         state.register().set_i(true);
//         state.interrupt.started = InterruptStatus::Executing;
//         4
//     }
//     fn name(&self) -> &'static str {
//         "IRQ"
//     }
// }

// pub(crate) struct InterruptBody;
// impl OpCode for InterruptBody {
//     fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
//         let new_pc = memory.read_u16(address, state);
//         state.register().set_pc(new_pc);
//         state.interrupt.started = InterruptStatus::Polling;
//         3
//     }
//     fn name(&self) -> &'static str {
//         "InterruptBody"
//     }
// }

struct AccStep1<
    FGet: Fn(&mut Register) -> u8,
    FSet: Fn(&mut Register, u8) -> (),
    FCalc: Fn(&mut Register, u8) -> u8,
> {
    getter: FGet,
    setter: FSet,
    calculator: FCalc,
}

impl<
        FGet: Fn(&mut Register) -> u8,
        FSet: Fn(&mut Register, u8) -> (),
        FCalc: Fn(&mut Register, u8) -> u8,
    > AccStep1<FGet, FSet, FCalc>
{
    pub fn new(getter: FGet, setter: FSet, calculator: FCalc) -> Self {
        Self {
            getter,
            setter,
            calculator,
        }
    }
}

impl<
        FGet: Fn(&mut Register) -> u8,
        FSet: Fn(&mut Register, u8) -> (),
        FCalc: Fn(&mut Register, u8) -> u8,
    > CpuStepState for AccStep1<FGet, FSet, FCalc>
{
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        // dummy read
        let pc = core.register.get_pc() as usize;
        let _ = core.memory.read(pc, ppu, cartridge, controller, apu, &mut core.interrupt);

        let data = (self.getter)(&mut core.register);
        let result = (self.calculator)(&mut core.register, data);
        (self.setter)(&mut core.register, result);

        FetchOpCode::new(&core.interrupt)
    }
}

struct MemStep1<FCalc: Fn(&mut Register, u8) -> u8> {
    address: usize,
    calculator: Option<FCalc>,
}

impl<FCalc: Fn(&mut Register, u8) -> u8> MemStep1<FCalc> {
    pub fn new(address: usize, calculator: FCalc) -> Self {
        Self {
            address,
            calculator: Some(calculator),
        }
    }
}

impl<FCalc: 'static + Fn(&mut Register, u8) -> u8> CpuStepState for MemStep1<FCalc> {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let data = core
            .memory
            .read(self.address, ppu, cartridge, controller, apu, &mut core.interrupt);
        Box::new(MemStep2::new(
            self.address,
            std::mem::replace(&mut self.calculator, None).unwrap(),
            data,
        ))
    }
}

struct MemStep2<FCalc: Fn(&mut Register, u8) -> u8> {
    address: usize,
    calculator: FCalc,
    data: u8,
}

impl<FCalc: Fn(&mut Register, u8) -> u8> MemStep2<FCalc> {
    pub fn new(address: usize, calculator: FCalc, data: u8) -> Self {
        Self {
            address,
            calculator,
            data,
        }
    }
}

impl<FCalc: Fn(&mut Register, u8) -> u8> CpuStepState for MemStep2<FCalc> {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        let result = (self.calculator)(&mut core.register, self.data);
        // dummy write
        core.memory
            .write(self.address, self.data, ppu, cartridge, controller, apu, &mut core.interrupt);
        Box::new(MemStep3::new(self.address, result))
    }
}

struct MemStep3 {
    address: usize,
    data: u8,
}

impl MemStep3 {
    pub fn new(address: usize, data: u8) -> Self {
        Self { address, data }
    }
}

impl CpuStepState for MemStep3 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Box<Cartridge>,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> Box<dyn CpuStepState> {
        core.memory
            .write(self.address, self.data, ppu, cartridge, controller, apu, &mut core.interrupt);
        FetchOpCode::new(&core.interrupt)
    }
}

pub(crate) struct Opcodes([Box<OpCode>; 256]);

impl Opcodes {
    pub fn new() -> Self {
        Opcodes([
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
        ])
    }

    pub fn get(&self, code: usize) -> &Box<OpCode> {
        &self.0[code]
    }
}
