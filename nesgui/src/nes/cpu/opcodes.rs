// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

fn compare(a: u8, state: &mut State, memory: &mut Memory, address: usize) {
    let b = memory.read(address, state);
    state.register().set_nz_from_value(a.wrapping_sub(b));
    state.register().set_c(a >= b);
}

fn push(state: &mut State, memory: &mut Memory, value: u8) {
    let sp = state.register().get_sp();
    state.stall += memory.write(0x100 | usize::from(sp), value, state);
    state.register().set_sp(sp.wrapping_sub(1));
}

fn pull(state: &mut State, memory: &mut Memory) -> u8 {
    let sp = state.register().get_sp().wrapping_add(1);
    state.register().set_sp(sp);
    memory.read(usize::from(sp) | 0x100, state)
}

fn push_u16(state: &mut State, memory: &mut Memory, value: u16) {
    let hi = (value >> 8) as u8;
    let low = (value & 0xFF) as u8;
    push(state, memory, hi);
    push(state, memory, low);
}

fn pull_u16(state: &mut State, memory: &mut Memory) -> u16 {
    let low = u16::from(pull(state, memory));
    let hi = u16::from(pull(state, memory));
    (hi << 8) | low
}

fn condition_jump(condition: bool, state: &mut State, address: usize) -> usize {
    if condition {
        let pc = state.register().get_pc() as usize;
        state.register().set_pc(address as u16);
        if page_crossed(address, pc) {
            3
        } else {
            2
        }
    } else {
        1
    }
}

pub(crate) trait OpCode {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize;
    fn name(&self) -> &'static str;
}

pub(crate) struct Adc;
impl OpCode for Adc {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let a = usize::from(state.register().get_a());
        let b = usize::from(memory.read(address, state));
        let c = if state.register().get_c() { 1 } else { 0 };
        let d = a + b + c;
        let result = (d & 0xFF) as u8;
        state.register().set_nz_from_value(result);
        state.register().set_a(result);
        state.register().set_c(d > 0xFF);
        state
            .register()
            .set_v((a ^ b) & 0x80 == 0 && (a ^ d) & 0x80 != 0);
        1
    }
    fn name(&self) -> &'static str {
        "ADC"
    }
}

pub(crate) struct And;
impl OpCode for And {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = state.register().get_a() & data;
        state.register().set_nz_from_value(result);
        state.register().set_a(result);
        1
    }
    fn name(&self) -> &'static str {
        "AND"
    }
}

fn asl(state: &mut State, data: u8) -> u8 {
    state.register().set_c(data & 0x80 != 0);
    let value = data << 1;
    state.register().set_nz_from_value(value);
    value
}

pub(crate) struct AslAcc;
impl OpCode for AslAcc {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let data = state.register().get_a();
        let result = asl(state, data);
        state.register().set_a(result);
        1
    }
    fn name(&self) -> &'static str {
        "ASL"
    }
}

pub(crate) struct AslMem;
impl OpCode for AslMem {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = asl(state, data);
        memory.write(address, data, state);
        state.stall += memory.write(address, result, state);
        3
    }
    fn name(&self) -> &'static str {
        "ASL"
    }
}

pub(crate) struct Bcc;
impl OpCode for Bcc {
    fn execute(&self, state: &mut State, _memory: &mut Memory, address: usize) -> usize {
        condition_jump(!state.register().get_c(), state, address)
    }
    fn name(&self) -> &'static str {
        "BCC"
    }
}

pub(crate) struct Bcs;
impl OpCode for Bcs {
    fn execute(&self, state: &mut State, _memory: &mut Memory, address: usize) -> usize {
        condition_jump(state.register().get_c(), state, address)
    }
    fn name(&self) -> &'static str {
        "BCS"
    }
}

pub(crate) struct Beq;
impl OpCode for Beq {
    fn execute(&self, state: &mut State, _memory: &mut Memory, address: usize) -> usize {
        condition_jump(state.register().get_z(), state, address)
    }
    fn name(&self) -> &'static str {
        "BEQ"
    }
}

pub(crate) struct Bit;
impl OpCode for Bit {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        state.register().set_v(data & 0x40 != 0);
        let a = data & state.register().get_a();
        state.register().set_z_from_value(a);
        state.register().set_n_from_value(data);
        1
    }
    fn name(&self) -> &'static str {
        "BIT"
    }
}

pub(crate) struct Bmi;
impl OpCode for Bmi {
    fn execute(&self, state: &mut State, _memory: &mut Memory, address: usize) -> usize {
        condition_jump(state.register().get_n(), state, address)
    }
    fn name(&self) -> &'static str {
        "BMI"
    }
}

pub(crate) struct Bne;
impl OpCode for Bne {
    fn execute(&self, state: &mut State, _memory: &mut Memory, address: usize) -> usize {
        condition_jump(!state.register().get_z(), state, address)
    }
    fn name(&self) -> &'static str {
        "BNE"
    }
}

pub(crate) struct Bpl;
impl OpCode for Bpl {
    fn execute(&self, state: &mut State, _memory: &mut Memory, address: usize) -> usize {
        condition_jump(!state.register().get_n(), state, address)
    }
    fn name(&self) -> &'static str {
        "BPL"
    }
}

pub(crate) struct Brk;
impl OpCode for Brk {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let pc = state.register().get_pc().wrapping_add(1);
        // state.register().set_b(true);
        push_u16(state, memory, pc);
        Php.execute(state, memory, address);
        state.register().set_i(true);
        state.interrupt.started = InterruptStatus::Executing;
        3
    }
    fn name(&self) -> &'static str {
        "BRK"
    }
}

pub(crate) struct Bvc;
impl OpCode for Bvc {
    fn execute(&self, state: &mut State, _memory: &mut Memory, address: usize) -> usize {
        condition_jump(!state.register().get_v(), state, address)
    }
    fn name(&self) -> &'static str {
        "BVC"
    }
}

pub(crate) struct Bvs;
impl OpCode for Bvs {
    fn execute(&self, state: &mut State, _memory: &mut Memory, address: usize) -> usize {
        condition_jump(state.register().get_v(), state, address)
    }
    fn name(&self) -> &'static str {
        "BVS"
    }
}

pub(crate) struct Clc;
impl OpCode for Clc {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        state.register().set_c(false);
        1
    }
    fn name(&self) -> &'static str {
        "CLC"
    }
}

pub(crate) struct Cld;
impl OpCode for Cld {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        state.register().set_d(false);
        1
    }
    fn name(&self) -> &'static str {
        "CLD"
    }
}

pub(crate) struct Cli;
impl OpCode for Cli {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        state.register().set_i(false);
        1
    }
    fn name(&self) -> &'static str {
        "CLI"
    }
}

pub(crate) struct Clv;
impl OpCode for Clv {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        state.register().set_v(false);
        1
    }
    fn name(&self) -> &'static str {
        "CLV"
    }
}

pub(crate) struct Cmp;
impl OpCode for Cmp {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        compare(state.register().get_a(), state, memory, address);
        1
    }
    fn name(&self) -> &'static str {
        "CMP"
    }
}

pub(crate) struct Cpx;
impl OpCode for Cpx {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        compare(state.register().get_x(), state, memory, address);
        1
    }
    fn name(&self) -> &'static str {
        "CPX"
    }
}

pub(crate) struct Cpy;
impl OpCode for Cpy {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        compare(state.register().get_y(), state, memory, address);
        1
    }
    fn name(&self) -> &'static str {
        "CPY"
    }
}

fn decrement(state: &mut State, data: u8) -> u8 {
    let result = data.wrapping_sub(1);
    state.register().set_nz_from_value(result);
    result
}

pub(crate) struct Dec;
impl OpCode for Dec {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = decrement(state, data);
        memory.write(address, data, state);
        state.stall += memory.write(address, result, state);
        3
    }
    fn name(&self) -> &'static str {
        "DEC"
    }
}

pub(crate) struct Dex;
impl OpCode for Dex {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let data = state.register().get_x();
        let result = decrement(state, data);
        state.register().set_x(result);
        1
    }
    fn name(&self) -> &'static str {
        "DEX"
    }
}

pub(crate) struct Dey;
impl OpCode for Dey {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let data = state.register().get_y();
        let result = decrement(state, data);
        state.register().set_y(result);
        1
    }
    fn name(&self) -> &'static str {
        "DEY"
    }
}

pub(crate) struct Eor;
impl OpCode for Eor {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = state.register().get_a() ^ data;
        state.register().set_nz_from_value(result);
        state.register().set_a(result);
        1
    }
    fn name(&self) -> &'static str {
        "EOR"
    }
}

fn increment(state: &mut State, data: u8) -> u8 {
    let result = data.wrapping_add(1);
    state.register().set_nz_from_value(result);
    result
}

pub(crate) struct Inc;
impl OpCode for Inc {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = increment(state, data);
        memory.write(address, data, state);
        state.stall += memory.write(address, result, state);
        3
    }
    fn name(&self) -> &'static str {
        "INC"
    }
}

pub(crate) struct Inx;
impl OpCode for Inx {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let data = state.register().get_x();
        let result = increment(state, data);
        state.register().set_x(result);
        1
    }
    fn name(&self) -> &'static str {
        "INX"
    }
}

pub(crate) struct Iny;
impl OpCode for Iny {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let data = state.register().get_y();
        let result = increment(state, data);
        state.register().set_y(result);
        1
    }
    fn name(&self) -> &'static str {
        "INY"
    }
}

pub(crate) struct Jmp;
impl OpCode for Jmp {
    fn execute(&self, state: &mut State, _memory: &mut Memory, address: usize) -> usize {
        state.register().set_pc(address as u16);
        0
    }
    fn name(&self) -> &'static str {
        "JMP"
    }
}

pub(crate) struct Jsr;
impl OpCode for Jsr {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let val = state.register().get_pc().wrapping_sub(1);
        push_u16(state, memory, val);
        state.register().set_pc(address as u16);
        3
    }
    fn name(&self) -> &'static str {
        "JSR"
    }
}

pub(crate) struct Lda;
impl OpCode for Lda {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        state.register().set_a(data);
        state.register().set_nz_from_value(data);
        1
    }
    fn name(&self) -> &'static str {
        "LDA"
    }
}

pub(crate) struct Ldx;
impl OpCode for Ldx {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        state.register().set_x(data);
        state.register().set_nz_from_value(data);
        1
    }
    fn name(&self) -> &'static str {
        "LDX"
    }
}

pub(crate) struct Ldy;
impl OpCode for Ldy {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        state.register().set_y(data);
        state.register().set_nz_from_value(data);
        1
    }
    fn name(&self) -> &'static str {
        "LDY"
    }
}

fn lsr(state: &mut State, data: u8) -> u8 {
    state.register().set_c(data & 0x01 != 0);
    let value = data >> 1;
    state.register().set_nz_from_value(value);
    value
}

pub(crate) struct LsrAcc;
impl OpCode for LsrAcc {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let data = state.register().get_a();
        let result = lsr(state, data);
        state.register().set_a(result);
        1
    }
    fn name(&self) -> &'static str {
        "LSR"
    }
}

pub(crate) struct LsrMem;
impl OpCode for LsrMem {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = lsr(state, data);
        memory.write(address, data, state);
        state.stall += memory.write(address, result, state);
        3
    }
    fn name(&self) -> &'static str {
        "LSR"
    }
}

pub(crate) struct Nop;
impl OpCode for Nop {
    fn execute(&self, _state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        1
    }
    fn name(&self) -> &'static str {
        "NOP"
    }
}

pub(crate) struct Ora;
impl OpCode for Ora {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = state.register().get_a() | data;
        state.register().set_nz_from_value(result);
        state.register().set_a(result);
        1
    }
    fn name(&self) -> &'static str {
        "ORA"
    }
}

pub(crate) struct Pha;
impl OpCode for Pha {
    fn execute(&self, state: &mut State, memory: &mut Memory, _address: usize) -> usize {
        let data = state.register().get_a();
        push(state, memory, data);
        2
    }
    fn name(&self) -> &'static str {
        "PHA"
    }
}

pub(crate) struct Php;
impl OpCode for Php {
    fn execute(&self, state: &mut State, memory: &mut Memory, _address: usize) -> usize {
        let data = state.register().get_p();
        push(state, memory, data | 0x10);
        2
    }
    fn name(&self) -> &'static str {
        "PHP"
    }
}

pub(crate) struct Pla;
impl OpCode for Pla {
    fn execute(&self, state: &mut State, memory: &mut Memory, _address: usize) -> usize {
        let result = pull(state, memory);
        state.register().set_a(result);
        state.register().set_nz_from_value(result);
        3
    }
    fn name(&self) -> &'static str {
        "PLA"
    }
}

pub(crate) struct Plp;
impl OpCode for Plp {
    fn execute(&self, state: &mut State, memory: &mut Memory, _address: usize) -> usize {
        let result = pull(state, memory);
        state.register().set_p((result & 0xEF) | 0x20);
        3
    }
    fn name(&self) -> &'static str {
        "PLP"
    }
}

fn rol(state: &mut State, data: u8) -> u8 {
    let c = if state.register().get_c() { 1 } else { 0 };
    state.register().set_c(data & 0x80 != 0);
    let value = data << 1 | c;
    state.register().set_nz_from_value(value);
    value
}

pub(crate) struct RolAcc;
impl OpCode for RolAcc {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let data = state.register().get_a();
        let result = rol(state, data);
        state.register().set_a(result);
        1
    }
    fn name(&self) -> &'static str {
        "ROL"
    }
}

pub(crate) struct RolMem;
impl OpCode for RolMem {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = rol(state, data);
        memory.write(address, data, state);
        state.stall += memory.write(address, result, state);
        3
    }
    fn name(&self) -> &'static str {
        "ROL"
    }
}

fn ror(state: &mut State, data: u8) -> u8 {
    let c = if state.register().get_c() { 0x80 } else { 0 };
    state.register().set_c(data & 0x01 != 0);
    let value = data >> 1 | c;
    state.register().set_nz_from_value(value);
    value
}

pub(crate) struct RorAcc;
impl OpCode for RorAcc {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let data = state.register().get_a();
        let result = ror(state, data);
        state.register().set_a(result);
        1
    }
    fn name(&self) -> &'static str {
        "ROR"
    }
}

pub(crate) struct RorMem;
impl OpCode for RorMem {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = ror(state, data);
        memory.write(address, data, state);
        state.stall += memory.write(address, result, state);
        3
    }
    fn name(&self) -> &'static str {
        "ROR"
    }
}

pub(crate) struct Rti;
impl OpCode for Rti {
    fn execute(&self, state: &mut State, memory: &mut Memory, _address: usize) -> usize {
        let result = pull(state, memory);
        state.register().set_p((result & 0xEF) | 0x20);
        let result = pull_u16(state, memory);
        state.register().set_pc(result);
        // 割り込み検出
        if state.interrupt.get_irq() && !state.register().get_i() {
            // state.interrupt.irq = false;
            state.interrupt.started = InterruptStatus::Detected;
        }
        5
    }
    fn name(&self) -> &'static str {
        "RTI"
    }
}

pub(crate) struct Rts;
impl OpCode for Rts {
    fn execute(&self, state: &mut State, memory: &mut Memory, _address: usize) -> usize {
        let result = pull_u16(state, memory);
        state.register().set_pc(result + 1);
        5
    }
    fn name(&self) -> &'static str {
        "RTS"
    }
}

pub(crate) struct Sbc;
impl OpCode for Sbc {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let a = u16::from(state.register().get_a());
        let b = u16::from(memory.read(address, state));
        let c = if state.register().get_c() { 0 } else { 1 };
        let d = a.wrapping_sub(b).wrapping_sub(c);
        let result = (d & 0xFF) as u8;
        state.register().set_nz_from_value(result);
        state.register().set_a(result);
        state.register().set_c(d <= 0xFF);
        state
            .register()
            .set_v((a ^ b) & 0x80 != 0 && (a ^ d) & 0x80 != 0);
        1
    }
    fn name(&self) -> &'static str {
        "SBC"
    }
}

pub(crate) struct Sec;
impl OpCode for Sec {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        state.register().set_c(true);
        1
    }
    fn name(&self) -> &'static str {
        "SEC"
    }
}

pub(crate) struct Sed;
impl OpCode for Sed {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        state.register().set_d(true);
        1
    }
    fn name(&self) -> &'static str {
        "SED"
    }
}

pub(crate) struct Sei;
impl OpCode for Sei {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        state.register().set_i(true);
        1
    }
    fn name(&self) -> &'static str {
        "SEI"
    }
}

pub(crate) struct Sta;
impl OpCode for Sta {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        state.stall += memory.write(address, state.register().get_a(), state);
        1
    }
    fn name(&self) -> &'static str {
        "STA"
    }
}

pub(crate) struct Stx;
impl OpCode for Stx {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        state.stall += memory.write(address, state.register().get_x(), state);
        1
    }
    fn name(&self) -> &'static str {
        "STX"
    }
}
pub(crate) struct Sty;
impl OpCode for Sty {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        state.stall += memory.write(address, state.register().get_y(), state);
        1
    }
    fn name(&self) -> &'static str {
        "STY"
    }
}

pub(crate) struct Tax;
impl OpCode for Tax {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let value = state.register().get_a();
        state.register().set_nz_from_value(value);
        state.register().set_x(value);
        1
    }
    fn name(&self) -> &'static str {
        "TAX"
    }
}
pub(crate) struct Tay;
impl OpCode for Tay {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let value = state.register().get_a();
        state.register().set_nz_from_value(value);
        state.register().set_y(value);
        1
    }
    fn name(&self) -> &'static str {
        "TAY"
    }
}

pub(crate) struct Tsx;
impl OpCode for Tsx {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let value = state.register().get_sp();
        state.register().set_nz_from_value(value);
        state.register().set_x(value);
        1
    }
    fn name(&self) -> &'static str {
        "TSX"
    }
}

pub(crate) struct Txa;
impl OpCode for Txa {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let value = state.register().get_x();
        state.register().set_nz_from_value(value);
        state.register().set_a(value);
        1
    }
    fn name(&self) -> &'static str {
        "TXA"
    }
}
pub(crate) struct Tya;
impl OpCode for Tya {
    fn execute(&self, state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        let value = state.register().get_y();
        state.register().set_nz_from_value(value);
        state.register().set_a(value);
        1
    }
    fn name(&self) -> &'static str {
        "TYA"
    }
}

pub(crate) struct Txs;
impl OpCode for Txs {
    fn execute(&self, state: &mut State, _: &mut Memory, _: usize) -> usize {
        let value = state.register().get_x();
        state.register().set_sp(value);
        1
    }
    fn name(&self) -> &'static str {
        "TXS"
    }
}

pub(crate) struct Nopd;
impl OpCode for Nopd {
    fn execute(&self, state: &mut State, _: &mut Memory, _: usize) -> usize {
        let pc = state.register().get_pc();
        state.register().set_pc(pc.wrapping_add(1));
        1
    }
    fn name(&self) -> &'static str {
        "NOPD"
    }
}

pub(crate) struct Nopi;
impl OpCode for Nopi {
    fn execute(&self, state: &mut State, _: &mut Memory, _: usize) -> usize {
        let pc = state.register().get_pc();
        state.register().set_pc(pc.wrapping_add(2));
        3
    }
    fn name(&self) -> &'static str {
        "NOPI"
    }
}

pub(crate) struct Lax;
impl OpCode for Lax {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        state.register().set_a(data);
        state.register().set_x(data);
        state.register().set_nz_from_value(data);
        1
    }
    fn name(&self) -> &'static str {
        "LAX"
    }
}

pub(crate) struct Sax;
impl OpCode for Sax {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = state.register().get_a() & state.register().get_x();
        state.stall += memory.write(address, data, state);
        1
    }
    fn name(&self) -> &'static str {
        "SAX"
    }
}

pub(crate) struct Dcp;
impl OpCode for Dcp {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let memdata = memory.read(address, state);
        let data = memdata.wrapping_sub(1);

        let a = state.register().get_a();
        state.register().set_nz_from_value(a.wrapping_sub(data));
        state.register().set_c(a >= data);

        memory.write(address, memdata, state);
        state.stall += memory.write(address, data, state);
        3
    }
    fn name(&self) -> &'static str {
        "DCP"
    }
}

pub(crate) struct Isc;
impl OpCode for Isc {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = increment(state, data);
        memory.write(address, data, state);
        state.stall += memory.write(address, result, state);

        let a = u16::from(state.register().get_a());
        let b = u16::from(result);
        let c = if state.register().get_c() { 0 } else { 1 };
        let d = a.wrapping_sub(b).wrapping_sub(c);
        let result = (d & 0xFF) as u8;
        state.register().set_nz_from_value(result);
        state.register().set_a(result);
        state.register().set_c(d <= 0xFF);
        state
            .register()
            .set_v((a ^ b) & 0x80 != 0 && (a ^ d) & 0x80 != 0);
        3
    }
    fn name(&self) -> &'static str {
        "ISC"
    }
}

pub(crate) struct Slo;
impl OpCode for Slo {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        state.register().set_c(data & 0x80 == 0x80);
        let a = state.register().get_a();
        let result = data << 1;
        state.register().set_a(a | result);
        state.register().set_nz_from_value(a | result);
        memory.write(address, data, state);
        state.stall += memory.write(address, result, state);
        3
    }
    fn name(&self) -> &'static str {
        "SLO"
    }
}

pub(crate) struct Rla;
impl OpCode for Rla {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let c = if state.register().get_c() { 1 } else { 0 };
        state.register().set_c(data & 0x80 != 0);
        let wd = (data << 1) | c;
        let value = wd & state.register().get_a();
        state.register().set_a(value);
        state.register().set_nz_from_value(value);
        memory.write(address, data, state);
        state.stall += memory.write(address, wd, state);
        3
    }
    fn name(&self) -> &'static str {
        "RLA"
    }
}

pub(crate) struct Sre;
impl OpCode for Sre {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let memdata = memory.read(address, state);
        state.register().set_c(memdata & 0x01 != 0);
        let data = memdata >> 1;
        let value = state.register().get_a() ^ data;
        state.register().set_a(value);
        state.register().set_nz_from_value(value);
        memory.write(address, memdata, state);
        state.stall += memory.write(address, data, state);
        3
    }
    fn name(&self) -> &'static str {
        "SRE"
    }
}

pub(crate) struct Rra;
impl OpCode for Rra {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = ror(state, data);
        memory.write(address, data, state);
        state.stall += memory.write(address, result, state);

        let a = usize::from(state.register().get_a());
        let b = usize::from(result);
        let c = if state.register().get_c() { 1 } else { 0 };
        let d = a + b + c;
        let result = (d & 0xFF) as u8;
        state.register().set_nz_from_value(result);
        state.register().set_a(result);
        state.register().set_c(d > 0xFF);
        state
            .register()
            .set_v((a ^ b) & 0x80 == 0 && (a ^ d) & 0x80 != 0);
        3
    }
    fn name(&self) -> &'static str {
        "RRA"
    }
}

pub(crate) struct Kil;
impl OpCode for Kil {
    fn execute(&self, _state: &mut State, _memory: &mut Memory, _address: usize) -> usize {
        //
        2
    }
    fn name(&self) -> &'static str {
        "KIL"
    }
}

pub(crate) struct Anc;
impl OpCode for Anc {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = state.register().get_a() & data;
        state.register().set_nz_from_value(result);
        state.register().set_c(result & 0x80 != 0);
        state.register().set_a(result);
        1
    }
    fn name(&self) -> &'static str {
        "ANC"
    }
}

pub(crate) struct Alr;
impl OpCode for Alr {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = state.register().get_a() & data;
        state.register().set_c(result & 0x01 != 0);
        let value = result >> 1;
        state.register().set_nz_from_value(value);
        state.register().set_a(value);
        1
    }
    fn name(&self) -> &'static str {
        "ALR"
    }
}

pub(crate) struct Ahx;
impl OpCode for Ahx {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let high = ((address >> 8) as u8).wrapping_add(1);
        let value = state.register().get_a() & state.register().get_x() & high;
        state.stall += memory.write(address, value, state);
        1
    }
    fn name(&self) -> &'static str {
        "AHX"
    }
}

pub(crate) struct Arr;
impl OpCode for Arr {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let a = state.register().get_a();
        let result = a & data;
        let c = if state.register().get_c() { 0x80 } else { 0 };
        state.register().set_c(result & 0x80 != 0);
        let value = result >> 1 | c;
        state
            .register()
            .set_v((((value >> 6) & 1) != 0) ^ (((value >> 5) & 1) != 0));
        state.register().set_nz_from_value(value);
        state.register().set_a(value);
        1
    }
    fn name(&self) -> &'static str {
        "ARR"
    }
}

pub(crate) struct Xaa;
impl OpCode for Xaa {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = state.register().get_x() & data;
        state.register().set_nz_from_value(result);
        state.register().set_a(result);
        1
    }
    fn name(&self) -> &'static str {
        "XAA"
    }
}

pub(crate) struct Tas;
impl OpCode for Tas {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let sp = state.register().get_a() & state.register().get_x();
        state.register().set_sp(sp);
        let pc_high = ((state.register().get_pc() >> 8) as u8).wrapping_add(1);
        state.stall += memory.write(address, sp & pc_high, state);
        1
    }
    fn name(&self) -> &'static str {
        "TAS"
    }
}

pub(crate) struct Shx;
impl OpCode for Shx {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let high = (address >> 8) as u8;
        let value = state.register().get_x() & high.wrapping_add(1);
        let new_addr = (usize::from(value) << 8) | (address & 0xFF);
        state.stall += memory.write(new_addr, value, state);
        1
    }
    fn name(&self) -> &'static str {
        "SHX"
    }
}

pub(crate) struct Shy;
impl OpCode for Shy {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let high = (address >> 8) as u8;
        let value = state.register().get_y() & high.wrapping_add(1);
        let new_addr = (usize::from(value) << 8) | (address & 0xFF);
        state.stall += memory.write(new_addr, value, state);
        1
    }
    fn name(&self) -> &'static str {
        "SHY"
    }
}

pub(crate) struct Las;
impl OpCode for Las {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let data = memory.read(address, state);
        let result = state.register().get_sp() & data;
        state.register().set_nz_from_value(result);
        state.register().set_a(result);
        state.register().set_x(result);
        state.register().set_sp(result);
        1
    }
    fn name(&self) -> &'static str {
        "LAS"
    }
}

pub(crate) struct Axs;
impl OpCode for Axs {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let a = u16::from(state.register().get_a() & state.register().get_x());
        let b = u16::from(memory.read(address, state));
        let d = a.wrapping_sub(b);

        let result = (d & 0xFF) as u8;
        state.register().set_nz_from_value(result);
        state.register().set_x(result);
        state.register().set_c(d <= 0xFF);

        1
    }
    fn name(&self) -> &'static str {
        "AXS"
    }
}

pub(crate) struct Nmi;
impl OpCode for Nmi {
    fn execute(&self, state: &mut State, memory: &mut Memory, _address: usize) -> usize {
        let pc = state.register().get_pc();
        let _ = memory.read(pc as usize, state); // dummy fetch
        state.register().set_b(false);
        push_u16(state, memory, pc);
        let data = state.register().get_p();
        push(state, memory, data);
        state.register().set_i(true);
        state.interrupt.started = InterruptStatus::Executing;
        4
    }
    fn name(&self) -> &'static str {
        "NMI"
    }
}

pub(crate) struct Irq;
impl OpCode for Irq {
    fn execute(&self, state: &mut State, memory: &mut Memory, _address: usize) -> usize {
        let pc = state.register().get_pc();
        let _ = memory.read(pc as usize, state); // dummy fetch
        state.register().set_b(false);

        push_u16(state, memory, pc);
        let data = state.register().get_p();
        push(state, memory, data);

        state.register().set_i(true);
        state.interrupt.started = InterruptStatus::Executing;
        4
    }
    fn name(&self) -> &'static str {
        "IRQ"
    }
}

pub(crate) struct InterruptBody;
impl OpCode for InterruptBody {
    fn execute(&self, state: &mut State, memory: &mut Memory, address: usize) -> usize {
        let new_pc = memory.read_u16(address, state);
        state.register().set_pc(new_pc);
        state.interrupt.started = InterruptStatus::Polling;
        3
    }
    fn name(&self) -> &'static str {
        "InterruptBody"
    }
}
