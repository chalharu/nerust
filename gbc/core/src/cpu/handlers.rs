//! Per-opcode M-cycle handlers.
//!
//! Each handler is called with the current step number (1-based).
//! It performs the work for that M-cycle and returns Continue or Exit.

use crate::cpu::registers::CpuRegisters;
use crate::cpu::{HandlerFn, Lr35902Cpu, StepResult};
use crate::memory::GbcMemoryBus;

pub fn build_table() -> [HandlerFn; 256] {
    let mut table: [HandlerFn; 256] = [h_invalid; 256];

    // ── Block 0 (0x00–0x3F) ───────────────────────────────
    table[0x00] = h_nop;
    table[0x01] = h_ld_r16_d16::<0>; // BC
    table[0x02] = h_ld_r16mem_a::<0>; // (BC)
    table[0x03] = h_inc_r16::<0>; // BC
    table[0x04] = h_inc_r8::<3>; // B
    table[0x05] = h_dec_r8::<3>; // B
    table[0x06] = h_ld_r8_d8::<3>; // B
    table[0x07] = h_rlca;
    table[0x08] = h_ld_a16_sp;
    table[0x09] = h_add_hl_r16::<0>; // BC
    table[0x0A] = h_ld_a_r16mem::<0>; // (BC)
    table[0x0B] = h_dec_r16::<0>; // BC
    table[0x0C] = h_inc_r8::<1>; // C
    table[0x0D] = h_dec_r8::<1>; // C
    table[0x0E] = h_ld_r8_d8::<1>; // C
    table[0x0F] = h_rrca;

    table[0x10] = h_stop;
    table[0x11] = h_ld_r16_d16::<2>; // DE
    table[0x12] = h_ld_r16mem_a::<2>; // (DE)
    table[0x13] = h_inc_r16::<2>; // DE
    table[0x14] = h_inc_r8::<5>; // D
    table[0x15] = h_dec_r8::<5>; // D
    table[0x16] = h_ld_r8_d8::<5>; // D
    table[0x17] = h_rla;
    table[0x18] = h_jr;
    table[0x19] = h_add_hl_r16::<2>; // DE
    table[0x1A] = h_ld_a_r16mem::<2>; // (DE)
    table[0x1B] = h_dec_r16::<2>; // DE
    table[0x1C] = h_inc_r8::<4>; // E
    table[0x1D] = h_dec_r8::<4>; // E
    table[0x1E] = h_ld_r8_d8::<4>; // E
    table[0x1F] = h_rra;

    for op in [0x20, 0x28, 0x30, 0x38] {
        table[op] = h_jr_cond::<0>;
    } // NZ/Z/NC/C
    table[0x21] = h_ld_r16_d16::<6>; // HL
    table[0x22] = h_ld_hli_a;
    table[0x23] = h_inc_r16::<6>; // HL
    table[0x24] = h_inc_r8::<7>; // H
    table[0x25] = h_dec_r8::<7>; // H
    table[0x26] = h_ld_r8_d8::<7>; // H
    table[0x27] = h_daa;
    table[0x29] = h_add_hl_r16::<6>; // HL
    table[0x2A] = h_ld_a_hli;
    table[0x2B] = h_dec_r16::<6>; // HL
    table[0x2C] = h_inc_r8::<6>; // L
    table[0x2D] = h_dec_r8::<6>; // L
    table[0x2E] = h_ld_r8_d8::<6>; // L
    table[0x2F] = h_cpl;
    table[0x31] = h_ld_r16_d16::<8>; // SP
    table[0x32] = h_ld_hld_a;
    table[0x33] = h_inc_sp;
    table[0x34] = h_inc_hl_indirect;
    table[0x35] = h_dec_hl_indirect;
    table[0x36] = h_ld_hl_d8;
    table[0x37] = h_scf;
    table[0x39] = h_add_hl_sp;
    table[0x3A] = h_ld_a_hld;
    table[0x3B] = h_dec_sp;
    table[0x3C] = h_inc_r8::<0>; // A
    table[0x3D] = h_dec_r8::<0>; // A
    table[0x3E] = h_ld_r8_d8::<0>; // A
    table[0x3F] = h_ccf;

    // ── Block 1 (0x40–0x7F): LD r8, r8 ──────────────────
    for op in 0x40..=0x7F {
        table[op] = h_ld_r8_r8;
    }
    table[0x76] = h_halt; // LD (HL),(HL) → HALT

    // ── Block 2 (0x80–0xBF): ALU A, r8 ──────────────────
    for op in 0x80..=0xBF {
        table[op] = h_alu_a_r8;
    }

    // ── Block 3 (0xC0–0xFF) ──────────────────────────────
    for op in [0xC0, 0xC8, 0xD0, 0xD8] {
        table[op] = h_ret_cond::<0>;
    } // NZ/Z/NC/C
    table[0xC1] = h_pop_r16::<0>; // BC
    for op in [0xC2, 0xCA, 0xD2, 0xDA] {
        table[op] = h_jp_cond::<0>;
    } // NZ/Z/NC/C
    table[0xC3] = h_jp_a16;
    for op in [0xC4, 0xCC, 0xD4, 0xDC] {
        table[op] = h_call_cond::<0>;
    } // NZ/Z/NC/C
    table[0xC5] = h_push_r16::<0>; // BC
    table[0xC6] = h_alu_a_d8::<0>; // ADD
    table[0xC7] = h_rst::<0>;
    table[0xC9] = h_ret;
    table[0xCB] = h_cb_prefix;
    table[0xCD] = h_call;
    table[0xCE] = h_alu_a_d8::<1>; // ADC
    table[0xCF] = h_rst::<1>;
    table[0xD1] = h_pop_r16::<2>; // DE
    table[0xD5] = h_push_r16::<2>; // DE
    table[0xD6] = h_alu_a_d8::<2>; // SUB
    table[0xD7] = h_rst::<2>;
    table[0xD9] = h_reti;
    table[0xDE] = h_alu_a_d8::<3>; // SBC
    table[0xDF] = h_rst::<3>;
    table[0xE0] = h_ldh_a8_a;
    table[0xE1] = h_pop_r16::<6>; // HL
    table[0xE2] = h_ld_c_a;
    table[0xE5] = h_push_r16::<6>; // HL
    table[0xE6] = h_alu_a_d8::<4>; // AND
    table[0xE7] = h_rst::<4>;
    table[0xE8] = h_add_sp_e;
    table[0xE9] = h_jp_hl;
    table[0xEA] = h_ld_a16_a;
    table[0xEE] = h_alu_a_d8::<5>; // XOR
    table[0xEF] = h_rst::<5>;
    table[0xF0] = h_ldh_a_a8;
    table[0xF1] = h_pop_r16::<7>; // AF
    table[0xF2] = h_ld_a_c;
    table[0xF3] = h_di;
    table[0xF5] = h_push_r16::<7>; // AF
    table[0xF6] = h_alu_a_d8::<6>; // OR
    table[0xF7] = h_rst::<6>;
    table[0xF8] = h_ld_hl_sp_e;
    table[0xF9] = h_ld_sp_hl;
    table[0xFA] = h_ld_a_a16;
    table[0xFB] = h_ei;
    table[0xFE] = h_alu_a_d8::<7>; // CP
    table[0xFF] = h_rst::<7>;

    table
}

// ── Helper: register index constants ──────────────────────
// For r8 operands encoded in opcode bits 0-2:
//   000=B, 001=C, 010=D, 011=E, 100=H, 101=L, 110=(HL), 111=A
// For r16 operands encoded in opcode bits 4-5:
//   00=BC, 01=DE, 10=HL, 11=SP (or AF for PUSH/POP)

const R8_B: u8 = 0;
const R8_C: u8 = 1;
const R8_D: u8 = 2;
const R8_E: u8 = 3;
const R8_H: u8 = 4;
const R8_L: u8 = 5;
const R8_HL: u8 = 6;
const R8_A: u8 = 7;

fn r8_from_opcode(opcode: u8) -> u8 {
    opcode & 0x07
}
fn r16_from_opcode(opcode: u8) -> u8 {
    (opcode >> 4) & 0x03
}

fn read_r8(core: &Lr35902Cpu, idx: u8) -> u8 {
    match idx {
        R8_B => core.registers.b,
        R8_C => core.registers.c,
        R8_D => core.registers.d,
        R8_E => core.registers.e,
        R8_H => core.registers.h,
        R8_L => core.registers.l,
        R8_A => core.registers.a,
        _ => 0,
    }
}

fn write_r8(core: &mut Lr35902Cpu, idx: u8, v: u8) {
    match idx {
        R8_B => core.registers.b = v,
        R8_C => core.registers.c = v,
        R8_D => core.registers.d = v,
        R8_E => core.registers.e = v,
        R8_H => core.registers.h = v,
        R8_L => core.registers.l = v,
        R8_A => core.registers.a = v,
        _ => {}
    }
}

fn r16_name(idx: u8) -> &'static str {
    match idx {
        0 => "BC",
        1 => "DE",
        2 => "HL",
        3 => "SP/AF",
        _ => "??",
    }
}

fn read_r16(core: &Lr35902Cpu, idx: u8) -> u16 {
    match idx {
        0 => core.registers.bc(),
        1 => core.registers.de(),
        2 => core.registers.hl(),
        3 => core.registers.sp,
        _ => 0,
    }
}

fn write_r16(core: &mut Lr35902Cpu, idx: u8, v: u16) {
    match idx {
        0 => core.registers.set_bc(v),
        1 => core.registers.set_de(v),
        2 => core.registers.set_hl(v),
        3 => core.registers.sp = v,
        _ => {}
    }
}

fn read_af(core: &Lr35902Cpu) -> u16 {
    core.registers.af()
}
fn write_af(core: &mut Lr35902Cpu, v: u16) {
    core.registers.set_af(v)
}

// ── Flag helpers ──────────────────────────────────────────

fn flags(core: &Lr35902Cpu) -> &CpuRegisters {
    &core.registers
}

fn inc8_result(v: u8) -> (u8, bool, bool) {
    let h = (v & 0x0F) == 0x0F;
    let r = v.wrapping_add(1);
    (r, r == 0, h)
}

fn dec8_result(v: u8) -> (u8, bool, bool) {
    let h = (v & 0x0F) == 0;
    let r = v.wrapping_sub(1);
    (r, r == 0, h)
}

fn add8_result(a: u8, v: u8) -> (u8, bool, bool, bool) {
    let r = a.wrapping_add(v);
    (
        r,
        r == 0,
        (a & 0x0F) + (v & 0x0F) > 0x0F,
        (a as u16) + (v as u16) > 0xFF,
    )
}

fn adc8_result(a: u8, v: u8, carry: bool) -> (u8, bool, bool, bool) {
    let c = carry as u8;
    let r = a.wrapping_add(v).wrapping_add(c);
    (
        r,
        r == 0,
        (a & 0x0F) + (v & 0x0F) + c > 0x0F,
        (a as u16) + (v as u16) + (c as u16) > 0xFF,
    )
}

fn sub8_result(a: u8, v: u8) -> (u8, bool, bool, bool) {
    let r = a.wrapping_sub(v);
    (r, r == 0, (a & 0x0F) < (v & 0x0F), a < v)
}

fn sbc8_result(a: u8, v: u8, carry: bool) -> (u8, bool, bool, bool) {
    let c = carry as u16;
    let r = a.wrapping_sub(v).wrapping_sub(carry as u8);
    (
        r,
        r == 0,
        (a & 0x0F) < ((v as u16 + c) as u8 & 0x0F),
        (a as u16) < (v as u16 + c),
    )
}

fn and8_result(a: u8, v: u8) -> (u8, bool) {
    (a & v, (a & v) == 0)
}

fn xor8_result(a: u8, v: u8) -> (u8, bool) {
    (a ^ v, (a ^ v) == 0)
}

fn or8_result(a: u8, v: u8) -> (u8, bool) {
    (a | v, (a | v) == 0)
}

fn cp8_result(a: u8, v: u8) -> (bool, bool, bool) {
    ((a & 0x0F) < (v & 0x0F), a < v, a.wrapping_sub(v) == 0)
}

fn add16_hl_result(hl: u16, v: u16) -> (bool, bool, u16) {
    (
        (hl & 0x0FFF) + (v & 0x0FFF) > 0x0FFF,
        (hl as u32) + (v as u32) > 0xFFFF,
        hl.wrapping_add(v),
    )
}

fn add16_sp(reg: &mut CpuRegisters, offset: i8) {
    let sp = reg.sp;
    let result = sp.wrapping_add_signed(offset as i16);
    reg.set_h((sp & 0x000F) + (offset as u8 as u16 & 0x000F) > 0x000F);
    reg.set_c((sp & 0x00FF) + (offset as u8 as u16 & 0x00FF) > 0x00FF);
    reg.set_z(false);
    reg.set_n(false);
    reg.sp = result;
}

fn ld_hl_sp_e(reg: &mut CpuRegisters, offset: i8) {
    let sp = reg.sp;
    let result = sp.wrapping_add_signed(offset as i16);
    reg.set_h((sp & 0x000F) + (offset as u8 as u16 & 0x000F) > 0x000F);
    reg.set_c((sp & 0x00FF) + (offset as u8 as u16 & 0x00FF) > 0x00FF);
    reg.set_z(false);
    reg.set_n(false);
    reg.set_hl(result);
}

// ── Stack helpers ─────────────────────────────────────────

fn push(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, v: u16) {
    core.registers.sp = core.registers.sp.wrapping_sub(1);
    bus.write(core.registers.sp, (v >> 8) as u8);
    core.registers.sp = core.registers.sp.wrapping_sub(1);
    bus.write(core.registers.sp, v as u8);
}

fn pop(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus) -> u16 {
    let lo = bus.read(core.registers.sp) as u16;
    core.registers.sp = core.registers.sp.wrapping_add(1);
    let hi = bus.read(core.registers.sp) as u16;
    core.registers.sp = core.registers.sp.wrapping_add(1);
    (hi << 8) | lo
}

// ── Opcode handlers ───────────────────────────────────────

fn h_nop(_: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
    StepResult::Exit
}

fn h_invalid(_: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
    StepResult::Exit
}

// INC r8 (1 M-cycle)
fn h_inc_r8<const R: u8>(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    let v = read_r8(core, R);
    let (r, z, h) = inc8_result(v);
    write_r8(core, R, r);
    core.registers.set_z(z);
    core.registers.set_n(false);
    core.registers.set_h(h);
    StepResult::Exit
}

// DEC r8 (1 M-cycle)
fn h_dec_r8<const R: u8>(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    let v = read_r8(core, R);
    let (r, z, h) = dec8_result(v);
    write_r8(core, R, r);
    core.registers.set_z(z);
    core.registers.set_n(true);
    core.registers.set_h(h);
    StepResult::Exit
}

// LD r8, d8 (2 M-cycles)
fn h_ld_r8_d8<const R: u8>(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            core.operand_count = 1;
            StepResult::Continue
        }
        2 => {
            write_r8(core, R, core.operands[0]);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// LD r8, r8 (1 M-cycle register-to-register, 2 M-cycles with (HL))
fn h_ld_r8_r8(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    let op = core.opcode;
    let src = r8_from_opcode(op);
    let dst = (op >> 3) & 0x07;
    if src == R8_HL {
        // LD r8, (HL)
        match step {
            1 => {
                core.operands[0] = bus.read(core.registers.hl());
                StepResult::Continue
            }
            2 => {
                write_r8(core, dst, core.operands[0]);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    } else if dst == R8_HL {
        // LD (HL), r8
        let v = read_r8(core, src);
        bus.write(core.registers.hl(), v);
        StepResult::Exit
    } else {
        // LD r8, r8
        let v = read_r8(core, src);
        write_r8(core, dst, v);
        StepResult::Exit
    }
}

// LD r16, d16 (3 M-cycles)
fn h_ld_r16_d16<const R: u8>(
    core: &mut Lr35902Cpu,
    bus: &mut GbcMemoryBus,
    step: u8,
) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => {
            core.operands[1] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        3 => {
            let v = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
            write_r16(core, R, v);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// LD (r16mem), A (2 M-cycles)
fn h_ld_r16mem_a<const R: u8>(
    core: &mut Lr35902Cpu,
    bus: &mut GbcMemoryBus,
    _step: u8,
) -> StepResult {
    let addr = read_r16(core, R);
    bus.write(addr, core.registers.a);
    StepResult::Exit
}

// LD A, (r16mem) (2 M-cycles)
fn h_ld_a_r16mem<const R: u8>(
    core: &mut Lr35902Cpu,
    bus: &mut GbcMemoryBus,
    _step: u8,
) -> StepResult {
    core.registers.a = bus.read(read_r16(core, R));
    StepResult::Exit
}

// INC/DEC r16 (2 M-cycles)
fn h_inc_r16<const R: u8>(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    write_r16(core, R, read_r16(core, R).wrapping_add(1));
    StepResult::Exit
}
fn h_dec_r16<const R: u8>(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    write_r16(core, R, read_r16(core, R).wrapping_sub(1));
    StepResult::Exit
}

// INC/DEC SP (2 M-cycles)
fn h_inc_sp(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    core.registers.sp = core.registers.sp.wrapping_add(1);
    StepResult::Exit
}
fn h_dec_sp(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    core.registers.sp = core.registers.sp.wrapping_sub(1);
    StepResult::Exit
}

// Rotates (1 M-cycle)
fn h_rlca(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    let c = core.registers.a & 0x80 != 0;
    core.registers.a = (core.registers.a << 1) | c as u8;
    core.registers.set_z(false);
    core.registers.set_n(false);
    core.registers.set_h(false);
    core.registers.set_c(c);
    StepResult::Exit
}
fn h_rrca(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    let c = core.registers.a & 0x01 != 0;
    core.registers.a = (core.registers.a >> 1) | if c { 0x80 } else { 0 };
    core.registers.set_z(false);
    core.registers.set_n(false);
    core.registers.set_h(false);
    core.registers.set_c(c);
    StepResult::Exit
}
fn h_rla(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    let c = core.registers.a & 0x80 != 0;
    core.registers.a = (core.registers.a << 1) | core.registers.c_flag() as u8;
    core.registers.set_z(false);
    core.registers.set_n(false);
    core.registers.set_h(false);
    core.registers.set_c(c);
    StepResult::Exit
}
fn h_rra(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    let c = core.registers.a & 0x01 != 0;
    core.registers.a = (core.registers.a >> 1) | if core.registers.c_flag() { 0x80 } else { 0 };
    core.registers.set_z(false);
    core.registers.set_n(false);
    core.registers.set_h(false);
    core.registers.set_c(c);
    StepResult::Exit
}

// DAA, CPL, SCF, CCF (1 M-cycle)
fn h_daa(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    let n = core.registers.n_flag();
    let h = core.registers.h_flag();
    let c = core.registers.c_flag();
    let mut a = core.registers.a;
    let mut carry = c;
    if n {
        if h {
            a = a.wrapping_sub(0x06);
        }
        if c {
            a = a.wrapping_sub(0x60);
        }
    } else {
        if h || (a & 0x0F) > 0x09 {
            a = a.wrapping_add(0x06);
        }
        if c || a > 0x99 {
            a = a.wrapping_add(0x60);
            carry = true;
        }
    }
    core.registers.a = a;
    core.registers.a = a;
    core.registers.set_z(a == 0);
    core.registers.set_h(false);
    core.registers.set_c(carry);
    StepResult::Exit
}
fn h_cpl(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _s: u8) -> StepResult {
    core.registers.a = !core.registers.a;
    core.registers.set_n(true);
    core.registers.set_h(true);
    StepResult::Exit
}
fn h_scf(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _s: u8) -> StepResult {
    core.registers.set_n(false);
    core.registers.set_h(false);
    core.registers.set_c(true);
    StepResult::Exit
}
fn h_ccf(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _s: u8) -> StepResult {
    core.registers.set_n(false);
    core.registers.set_h(false);
    {
        let c = core.registers.c_flag();
        core.registers.set_c(!c);
    };
    StepResult::Exit
}

// JR e (3 M-cycles)
fn h_jr(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => StepResult::Continue,
        3 => {
            core.registers.pc = core
                .registers
                .pc
                .wrapping_add_signed(core.operands[0] as i8 as i16);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// JR cond (NZ=0, Z=1, NC=2, C=3 via opcode bits 3-4)
fn h_jr_cond<const C: u8>(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    let taken = match C {
        0 => !core.registers.z_flag(),
        1 => core.registers.z_flag(),
        2 => !core.registers.c_flag(),
        _ => core.registers.c_flag(),
    };
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => {
            if !taken {
                StepResult::Exit
            } else {
                StepResult::Continue
            }
        }
        3 => {
            core.registers.pc = core
                .registers
                .pc
                .wrapping_add_signed(core.operands[0] as i8 as i16);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// JP a16 (4 M-cycles)
fn h_jp_a16(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => {
            core.operands[1] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        3 => StepResult::Continue,
        4 => {
            core.registers.pc = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// JP cond (3-4 M-cycles)
fn h_jp_cond<const C: u8>(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    let taken = match C {
        0 => !core.registers.z_flag(),
        1 => core.registers.z_flag(),
        2 => !core.registers.c_flag(),
        _ => core.registers.c_flag(),
    };
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => {
            core.operands[1] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        3 => {
            if !taken {
                StepResult::Exit
            } else {
                StepResult::Continue
            }
        }
        4 => {
            core.registers.pc = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// JP HL (1 M-cycle)
fn h_jp_hl(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    core.registers.pc = core.registers.hl();
    StepResult::Exit
}

// CALL (6 M-cycles)
fn h_call(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => {
            core.operands[1] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        3 | 4 => StepResult::Continue,
        5 => {
            let _addr = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
            let ret = core.registers.pc;
            core.registers.sp = core.registers.sp.wrapping_sub(1);
            bus.write(core.registers.sp, (ret >> 8) as u8);
            StepResult::Continue
        }
        6 => {
            core.registers.sp = core.registers.sp.wrapping_sub(1);
            bus.write(core.registers.sp, core.registers.pc as u8);
            let addr = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
            core.registers.pc = addr;
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// CALL cond (3-6 M-cycles)
fn h_call_cond<const C: u8>(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    let taken = match C {
        0 => !core.registers.z_flag(),
        1 => core.registers.z_flag(),
        2 => !core.registers.c_flag(),
        _ => core.registers.c_flag(),
    };
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => {
            core.operands[1] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        3 => {
            if !taken {
                StepResult::Exit
            } else {
                StepResult::Continue
            }
        }
        4 => StepResult::Continue,
        5 => {
            let ret = core.registers.pc;
            core.registers.sp = core.registers.sp.wrapping_sub(1);
            bus.write(core.registers.sp, (ret >> 8) as u8);
            StepResult::Continue
        }
        6 => {
            core.registers.sp = core.registers.sp.wrapping_sub(1);
            bus.write(core.registers.sp, core.registers.pc as u8);
            let addr = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
            core.registers.pc = addr;
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// RET (4 M-cycles)
fn h_ret(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 | 2 => StepResult::Continue,
        3 => {
            let lo = bus.read(core.registers.sp) as u16;
            core.registers.sp = core.registers.sp.wrapping_add(1);
            core.operands[0] = lo as u8;
            StepResult::Continue
        }
        4 => {
            let hi = bus.read(core.registers.sp) as u16;
            core.registers.sp = core.registers.sp.wrapping_add(1);
            core.registers.pc = (hi << 8) | core.operands[0] as u16;
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// RET cond (2-5 M-cycles)
fn h_ret_cond<const C: u8>(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    let taken = match C {
        0 => !core.registers.z_flag(),
        1 => core.registers.z_flag(),
        2 => !core.registers.c_flag(),
        _ => core.registers.c_flag(),
    };
    match step {
        1 => {
            if !taken {
                StepResult::Exit
            } else {
                StepResult::Continue
            }
        }
        2 => StepResult::Continue,
        3 => {
            let lo = bus.read(core.registers.sp) as u16;
            core.registers.sp = core.registers.sp.wrapping_add(1);
            core.operands[0] = lo as u8;
            StepResult::Continue
        }
        4 => StepResult::Continue,
        5 => {
            let hi = bus.read(core.registers.sp) as u16;
            core.registers.sp = core.registers.sp.wrapping_add(1);
            core.registers.pc = (hi << 8) | core.operands[0] as u16;
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// RETI (4 M-cycles)
fn h_reti(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 | 2 => StepResult::Continue,
        3 => {
            let lo = bus.read(core.registers.sp) as u16;
            core.registers.sp = core.registers.sp.wrapping_add(1);
            core.operands[0] = lo as u8;
            StepResult::Continue
        }
        4 => {
            let hi = bus.read(core.registers.sp) as u16;
            core.registers.sp = core.registers.sp.wrapping_add(1);
            core.registers.pc = (hi << 8) | core.operands[0] as u16;
            bus.set_ime(true);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// RST (4 M-cycles)
fn h_rst<const V: u8>(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    let addr = V as u16 * 8;
    match step {
        1 | 2 => StepResult::Continue,
        3 => {
            core.registers.sp = core.registers.sp.wrapping_sub(1);
            bus.write(core.registers.sp, (core.registers.pc >> 8) as u8);
            StepResult::Continue
        }
        4 => {
            core.registers.sp = core.registers.sp.wrapping_sub(1);
            bus.write(core.registers.sp, core.registers.pc as u8);
            core.registers.pc = addr;
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// PUSH (4 M-cycles)
fn h_push_r16<const R: u8>(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    let v = if R == 3 {
        read_af(core)
    } else {
        read_r16(core, R)
    };
    match step {
        1 => StepResult::Continue,
        2 => StepResult::Continue,
        3 => {
            core.registers.sp = core.registers.sp.wrapping_sub(1);
            bus.write(core.registers.sp, (v >> 8) as u8);
            StepResult::Continue
        }
        4 => {
            core.registers.sp = core.registers.sp.wrapping_sub(1);
            bus.write(core.registers.sp, v as u8);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// POP (3 M-cycles)
fn h_pop_r16<const R: u8>(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = bus.read(core.registers.sp);
            core.registers.sp = core.registers.sp.wrapping_add(1);
            StepResult::Continue
        }
        2 => {
            core.operands[1] = bus.read(core.registers.sp);
            core.registers.sp = core.registers.sp.wrapping_add(1);
            StepResult::Continue
        }
        3 => {
            let v = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
            if R == 3 {
                write_af(core, v)
            } else {
                write_r16(core, R, v)
            }
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// ALU A, r8 (1-2 M-cycles)
fn h_alu_a_r8(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    let op = core.opcode;
    let src = r8_from_opcode(op);
    let alu_op = (op >> 3) & 0x07; // 0=ADD, 1=ADC, 2=SUB, 3=SBC, 4=AND, 5=XOR, 6=OR, 7=CP
    if src == R8_HL {
        match step {
            1 => {
                core.operands[0] = bus.read(core.registers.hl());
                StepResult::Continue
            }
            2 => {
                alu_op_r8(core, alu_op, core.operands[0]);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    } else {
        let v = read_r8(core, src);
        alu_op_r8(core, alu_op, v);
        StepResult::Exit
    }
}

// ALU A, d8 (2 M-cycles)
fn h_alu_a_d8<const OP: u8>(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => {
            alu_op_r8(core, OP, core.operands[0]);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

fn alu_op_r8(core: &mut Lr35902Cpu, op: u8, v: u8) {
    match op {
        0 => {
            let a = core.registers.a;
            let (r, z, h, c) = add8_result(a, v);
            core.registers.a = r;
            core.registers.set_z(z);
            core.registers.set_h(h);
            core.registers.set_c(c);
            core.registers.set_n(false);
        }
        1 => {
            let a = core.registers.a;
            let c_flag = core.registers.c_flag();
            let (r, z, h, c) = adc8_result(a, v, c_flag);
            core.registers.a = r;
            core.registers.set_z(z);
            core.registers.set_h(h);
            core.registers.set_c(c);
            core.registers.set_n(false);
        }
        2 => {
            let a = core.registers.a;
            let (r, z, h, c) = sub8_result(a, v);
            core.registers.a = r;
            core.registers.set_z(z);
            core.registers.set_h(h);
            core.registers.set_c(c);
            core.registers.set_n(true);
        }
        3 => {
            let a = core.registers.a;
            let c_flag = core.registers.c_flag();
            let (r, z, h, c) = sbc8_result(a, v, c_flag);
            core.registers.a = r;
            core.registers.set_z(z);
            core.registers.set_h(h);
            core.registers.set_c(c);
            core.registers.set_n(true);
        }
        4 => {
            let (r, z) = and8_result(core.registers.a, v);
            core.registers.a = r;
            core.registers.set_z(z);
            core.registers.set_n(false);
            core.registers.set_h(true);
            core.registers.set_c(false);
        }
        5 => {
            let (r, z) = xor8_result(core.registers.a, v);
            core.registers.a = r;
            core.registers.set_z(z);
            core.registers.set_n(false);
            core.registers.set_h(false);
            core.registers.set_c(false);
        }
        6 => {
            let (r, z) = or8_result(core.registers.a, v);
            core.registers.a = r;
            core.registers.set_z(z);
            core.registers.set_n(false);
            core.registers.set_h(false);
            core.registers.set_c(false);
        }
        7 => {
            let a = core.registers.a;
            let (h, c, z) = cp8_result(a, v);
            core.registers.set_h(h);
            core.registers.set_c(c);
            core.registers.set_z(z);
            core.registers.set_n(true);
        }
        _ => {}
    }
}

// LD (HL+), A / LD A, (HL+) / LD (HL-), A / LD A, (HL-) (2 M-cycles)
fn h_ld_hli_a(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    let addr = core.registers.hl();
    bus.write(addr, core.registers.a);
    core.registers.set_hl(addr.wrapping_add(1));
    StepResult::Exit
}
fn h_ld_a_hli(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    let addr = core.registers.hl();
    core.registers.a = bus.read(addr);
    core.registers.set_hl(addr.wrapping_add(1));
    StepResult::Exit
}
fn h_ld_hld_a(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    let addr = core.registers.hl();
    bus.write(addr, core.registers.a);
    core.registers.set_hl(addr.wrapping_sub(1));
    StepResult::Exit
}
fn h_ld_a_hld(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    let addr = core.registers.hl();
    core.registers.a = bus.read(addr);
    core.registers.set_hl(addr.wrapping_sub(1));
    StepResult::Exit
}

// ADD HL, r16 (2 M-cycles)
fn h_add_hl_r16<const R: u8>(
    core: &mut Lr35902Cpu,
    _bus: &mut GbcMemoryBus,
    step: u8,
) -> StepResult {
    match step {
        1 => StepResult::Continue,
        2 => {
            let hl = core.registers.hl();
            let v = read_r16(core, R);
            let (h, c, r) = add16_hl_result(hl, v);
            core.registers.set_hl(r);
            core.registers.set_h(h);
            core.registers.set_c(c);
            core.registers.set_n(false);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// ADD HL, SP (2 M-cycles)
fn h_add_hl_sp(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => StepResult::Continue,
        2 => {
            let hl = core.registers.hl();
            let sp = core.registers.sp;
            let (h, c, r) = add16_hl_result(hl, sp);
            core.registers.set_hl(r);
            core.registers.set_h(h);
            core.registers.set_c(c);
            core.registers.set_n(false);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// INC/DEC (HL) — 3 M-cycles
fn h_inc_hl_indirect(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    let addr = core.registers.hl();
    match step {
        1 => {
            let v = bus.read(addr);
            core.operands[0] = v;
            StepResult::Continue
        }
        2 => StepResult::Continue,
        3 => {
            let v = core.operands[0];
            let (r, z, h) = inc8_result(v);
            bus.write(addr, r);
            core.registers.set_z(z);
            core.registers.set_n(false);
            core.registers.set_h(h);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}
fn h_dec_hl_indirect(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    let addr = core.registers.hl();
    match step {
        1 => {
            let v = bus.read(addr);
            core.operands[0] = v;
            StepResult::Continue
        }
        2 => StepResult::Continue,
        3 => {
            let v = core.operands[0];
            let (r, z, h) = dec8_result(v);
            bus.write(addr, r);
            core.registers.set_z(z);
            core.registers.set_n(true);
            core.registers.set_h(h);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// LD (HL), d8 (3 M-cycles)
fn h_ld_hl_d8(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => StepResult::Continue,
        3 => {
            bus.write(core.registers.hl(), core.operands[0]);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// LDH (a8), A / LDH A, (a8) (3 M-cycles)
fn h_ldh_a8_a(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => StepResult::Continue,
        3 => {
            bus.write(0xFF00 | core.operands[0] as u16, core.registers.a);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}
fn h_ldh_a_a8(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => StepResult::Continue,
        3 => {
            core.registers.a = bus.read(0xFF00 | core.operands[0] as u16);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// LD (C), A / LD A, (C) (2 M-cycles)
fn h_ld_c_a(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    bus.write(0xFF00 | core.registers.c as u16, core.registers.a);
    StepResult::Exit
}
fn h_ld_a_c(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    core.registers.a = bus.read(0xFF00 | core.registers.c as u16);
    StepResult::Exit
}

// LD (a16), A / LD A, (a16) (4 M-cycles)
fn h_ld_a16_a(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => {
            core.operands[1] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        3 => StepResult::Continue,
        4 => {
            let addr = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
            bus.write(addr, core.registers.a);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}
fn h_ld_a_a16(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => {
            core.operands[1] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        3 => StepResult::Continue,
        4 => {
            let addr = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
            core.registers.a = bus.read(addr);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// LD (a16), SP (5 M-cycles)
fn h_ld_a16_sp(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => {
            core.operands[1] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        3 | 4 => StepResult::Continue,
        5 => {
            let addr = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
            bus.write(addr, core.registers.sp as u8);
            bus.write(addr.wrapping_add(1), (core.registers.sp >> 8) as u8);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// ADD SP, e (4 M-cycles)
fn h_add_sp_e(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 | 3 => StepResult::Continue,
        4 => {
            let offset = core.operands[0] as i8;
            let sp = core.registers.sp;
            let result = sp.wrapping_add_signed(offset as i16);
            core.registers
                .set_h((sp & 0x000F) + (offset as u8 as u16 & 0x000F) > 0x000F);
            core.registers
                .set_c((sp & 0x00FF) + (offset as u8 as u16 & 0x00FF) > 0x00FF);
            core.registers.set_z(false);
            core.registers.set_n(false);
            core.registers.sp = result;
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// LD HL, SP+e (3 M-cycles)
fn h_ld_hl_sp_e(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => StepResult::Continue,
        3 => {
            let offset = core.operands[0] as i8;
            let sp = core.registers.sp;
            let result = sp.wrapping_add_signed(offset as i16);
            core.registers
                .set_h((sp & 0x000F) + (offset as u8 as u16 & 0x000F) > 0x000F);
            core.registers
                .set_c((sp & 0x00FF) + (offset as u8 as u16 & 0x00FF) > 0x00FF);
            core.registers.set_z(false);
            core.registers.set_n(false);
            core.registers.set_hl(result);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// LD SP, HL (2 M-cycles)
fn h_ld_sp_hl(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => StepResult::Continue,
        2 => {
            core.registers.sp = core.registers.hl();
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// DI / EI (1 M-cycle)
fn h_di(_core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    bus.set_ime(false);
    StepResult::Exit
}
fn h_ei(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    core.ime_delayed = true;
    StepResult::Exit
}

// HALT (1 M-cycle)
fn h_halt(_core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    bus.halt_cpu();
    StepResult::Exit
}

// STOP (1 M-cycle)
fn h_stop(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
    core.registers.pc = core.registers.pc.wrapping_add(1);
    bus.stop();
    StepResult::Exit
}

// CB prefix (2+ M-cycles)
fn h_cb_prefix(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    match step {
        1 => {
            core.operands[0] = core.fetch_pc_byte(bus);
            StepResult::Continue
        }
        2 => {
            cb_execute(core, bus);
            StepResult::Exit
        }
        _ => unreachable!(),
    }
}

// ── CB-prefix execution ───────────────────────────────────

fn cb_execute(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus) {
    let op = core.operands[0];
    let reg_idx = op & 0x07;
    let group = (op >> 3) & 0x07;
    let _bit = ((op >> 3) & 0x07);
    let is_hl = reg_idx == R8_HL;
    let r = if is_hl {
        bus.read(core.registers.hl())
    } else {
        read_r8(core, reg_idx)
    };

    let f = core.registers;
    let (result, carry) = match group {
        0 => cb_rlc(r),
        1 => cb_rrc(r),
        2 => cb_rl(r, f.c_flag()),
        3 => cb_rr(r, f.c_flag()),
        4 => cb_sla(r),
        5 => cb_sra(r),
        6 => cb_swap(r),
        7 => cb_srl(r),
        _ => (r, false),
    };

    // BIT operation (opcodes 0x40-0x7F)
    if (op & 0xC0) == 0x40 {
        let test_bit = (op >> 3) & 0x07;
        core.registers.set_z(r & (1 << test_bit) == 0);
        core.registers.set_n(false);
        core.registers.set_h(true);
        return;
    }

    // RES operation (opcodes 0x80-0xBF)
    if (op & 0xC0) == 0x80 {
        let res_bit = (op >> 3) & 0x07;
        let v = r & !(1 << res_bit);
        if is_hl {
            bus.write(core.registers.hl(), v);
        } else {
            write_r8(core, reg_idx, v);
        }
        return;
    }

    // SET operation (opcodes 0xC0-0xFF)
    if (op & 0xC0) == 0xC0 {
        let set_bit = (op >> 3) & 0x07;
        let v = r | (1 << set_bit);
        if is_hl {
            bus.write(core.registers.hl(), v);
        } else {
            write_r8(core, reg_idx, v);
        }
        return;
    }

    // Shift/rotate: write result and set flags (except BIT/RES/SET above)
    core.registers.set_z(result == 0);
    core.registers.set_n(false);
    core.registers.set_h(false);
    core.registers.set_c(carry);
    if is_hl {
        bus.write(core.registers.hl(), result);
    } else {
        write_r8(core, reg_idx, result);
    }
}

fn cb_rlc(v: u8) -> (u8, bool) {
    let c = v & 0x80 != 0;
    ((v << 1) | c as u8, c)
}
fn cb_rrc(v: u8) -> (u8, bool) {
    let c = v & 0x01 != 0;
    ((v >> 1) | if c { 0x80 } else { 0 }, c)
}
fn cb_rl(v: u8, old_c: bool) -> (u8, bool) {
    let c = v & 0x80 != 0;
    ((v << 1) | old_c as u8, c)
}
fn cb_rr(v: u8, old_c: bool) -> (u8, bool) {
    let c = v & 0x01 != 0;
    ((v >> 1) | if old_c { 0x80 } else { 0 }, c)
}
fn cb_sla(v: u8) -> (u8, bool) {
    let c = v & 0x80 != 0;
    (v << 1, c)
}
fn cb_sra(v: u8) -> (u8, bool) {
    let c = v & 0x01 != 0;
    ((v >> 1) | (v & 0x80), c)
}
fn cb_swap(v: u8) -> (u8, bool) {
    (v.rotate_right(4), false)
}
fn cb_srl(v: u8) -> (u8, bool) {
    let c = v & 0x01 != 0;
    (v >> 1, c)
}
