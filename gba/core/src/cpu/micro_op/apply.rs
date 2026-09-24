//! Micro-op execution: shared memory/ALU helpers, Thumb commits,
//! multiply, and the [`apply_op`] dispatcher. ARM commit helpers
//! (SWP/PSR/DP/traps) live in [`apply_arm`]; this file calls down
//! into it, never the reverse.
use super::apply_arm::{apply_dp_reg, apply_psr, apply_swp, apply_trap_swi, apply_trap_und};
use super::{
    AluEffect, AluImmOp, BlockEmptyEffect, BlockEndEffect, BlockWord, MemAccess, MicroOp,
    MulEffect, PcRelRead,
};
use crate::cpu::semantics::{
    barrel_shift_register, multiplier_cycles, multiplier_cycles_long, multiply_64,
    multiply_carry_hi, multiply_carry_lo, multiply_tick_full, register_pair, update_nz,
};
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

fn resolve_addr(regs: &CpuRegisters, a: MemAccess) -> (u32, u32) {
    let base = if a.is_sp { regs.sp() } else { regs.r(a.rn) };
    let offset = a
        .offset_register
        .map_or(a.offset, |register| regs.r(register));
    let adjusted = if a.subtract {
        base.wrapping_sub(offset)
    } else {
        base.wrapping_add(offset)
    };
    (if a.post_indexed { base } else { adjusted }, adjusted)
}

/// Apply one data access with the exact bus-call order
/// (access, writeback, then fetch-stream-break). The issue clock (+1)
/// lands at the call site; the bus calls charge into `access_wait_cycles`.
fn apply_read(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, a: MemAccess) {
    let (addr, writeback) = resolve_addr(regs, a);
    let v = match (a.width, a.signed_load) {
        (4, _) => bus.read32(addr),
        (2, false) => bus.read_ldr_halfword(addr),
        (2, true) if addr & 1 != 0 => {
            if a.halfword_odd_quirk {
                // Thumb LDRSH odd-address bus quirk (see field docs).
                let half = bus.read_ldr_halfword(addr & !1) & 0xFFFF;
                bus.merge_rom_half(addr, half);
                ((half >> 8) as u8) as i8 as i32 as u32
            } else {
                // ARM LDRSH at odd addresses is a plain sign-extended
                // byte read.
                bus.read8(addr) as i8 as i32 as u32
            }
        }
        (2, true) => bus.read16(addr) as i16 as i32 as u32,
        (1, true) => bus.read8(addr) as i8 as i32 as u32,
        (1, false) => u32::from(bus.read8(addr)),
        _ => unreachable!(),
    };
    regs.set_r(a.rd, v);
    if a.writeback && a.rd != a.rn {
        regs.set_r(a.rn, writeback);
    }
    bus.charge_fetch_stream_break(addr);
}

/// Apply one data store with the exact bus-call order
/// (access, then fetch-stream-break). The issue clock (+1) lands at the
/// call site; the bus calls charge into `access_wait_cycles`.
fn apply_write(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, a: MemAccess) {
    let (addr, writeback) = resolve_addr(regs, a);
    let value = a.store_value.unwrap_or_else(|| regs.r(a.rd));
    match a.width {
        4 => bus.write32(addr, value),
        2 => bus.write16(addr, value as u16),
        1 => bus.write8(addr, value as u8),
        _ => unreachable!(),
    }
    if a.writeback {
        regs.set_r(a.rn, writeback);
    }
    bus.charge_fetch_stream_break(addr);
}

fn apply_alu(regs: &mut CpuRegisters, fx: AluEffect) {
    // Shifter carry-out: snapshot for S with a rotated immediate, else
    // the live CPSR carry. Arithmetic carry-INS always read the live
    // CPSR carry (the carry-ins stay separate; conflating
    // them breaks ADC/SBC/RSC with rotation).
    let cpsr_carry = regs.cpsr_c();
    let shift_carry = fx.carry.unwrap_or(cpsr_carry);
    let rn_val = regs.r(fx.rn);
    let (result, carry, overflow) = match fx.op {
        AluImmOp::Mov => (fx.imm, shift_carry, false),
        AluImmOp::Mvn => (!fx.imm, shift_carry, false),
        AluImmOp::And | AluImmOp::Tst => (rn_val & fx.imm, shift_carry, false),
        AluImmOp::Eor | AluImmOp::Teq => (rn_val ^ fx.imm, shift_carry, false),
        AluImmOp::Orr => (rn_val | fx.imm, shift_carry, false),
        AluImmOp::Bic => (rn_val & !fx.imm, shift_carry, false),
        AluImmOp::Add | AluImmOp::Cmn => {
            let (r, c) = rn_val.overflowing_add(fx.imm);
            let v = ((!(rn_val ^ fx.imm)) & (rn_val ^ r) & 0x8000_0000) != 0;
            (r, c, v)
        }
        AluImmOp::Sub | AluImmOp::Cmp => {
            let (r, b) = rn_val.overflowing_sub(fx.imm);
            let v = ((rn_val ^ fx.imm) & (rn_val ^ r) & 0x8000_0000) != 0;
            (r, !b, v)
        }
        AluImmOp::Rsb => {
            let (r, b) = fx.imm.overflowing_sub(rn_val);
            let v = ((fx.imm ^ rn_val) & (fx.imm ^ r) & 0x8000_0000) != 0;
            (r, !b, v)
        }
        AluImmOp::Adc => {
            let c_in = u32::from(cpsr_carry);
            let (r1, c1) = rn_val.overflowing_add(fx.imm);
            let (r, c2) = r1.overflowing_add(c_in);
            // Overflow via the exact signed total (a
            // two-stage OR diverges on borrow chains).
            let signed = rn_val as i32 as i64 + fx.imm as i32 as i64 + i64::from(c_in);
            let v = signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN);
            (r, c1 || c2, v)
        }
        AluImmOp::Sbc => {
            // SBC = Rn - imm - !C.
            let not_c = 1 - u32::from(cpsr_carry);
            let (r1, b1) = rn_val.overflowing_sub(fx.imm);
            let (r, b2) = r1.overflowing_sub(not_c);
            let signed = rn_val as i32 as i64 - fx.imm as i32 as i64 - i64::from(not_c);
            let v = signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN);
            (r, !(b1 || b2), v)
        }
        AluImmOp::Rsc => {
            // RSC = imm - Rn - !C.
            let not_c = 1 - u32::from(cpsr_carry);
            let (r1, b1) = fx.imm.overflowing_sub(rn_val);
            let (r, b2) = r1.overflowing_sub(not_c);
            let signed = fx.imm as i32 as i64 - rn_val as i32 as i64 - i64::from(not_c);
            let v = signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN);
            (r, !(b1 || b2), v)
        }
    };
    if !fx.op.is_flag_only() {
        regs.set_r(fx.rd, result);
    }
    if fx.set_flags {
        // USR/SYS have no SPSR: exception-return restores only apply in
        // modes with an SPSR bank (mode read precedes any change here).
        let has_spsr = !matches!(regs.cpsr_mode(), 0x10 | 0x1F);
        let rd_pc_write = fx.rd == 15 && !fx.op.is_flag_only();
        // S with Rd==15 restores CPSR (flag-only forms without touching
        // PC; writes via `write_result` above); a privileged Rd==15
        // write then skips the flag update (CPSR replaced wholesale).
        if fx.rd == 15 && has_spsr {
            regs.set_cpsr(regs.spsr());
            if fx.op.is_flag_only() || rd_pc_write {
                return;
            }
        }
        if !(rd_pc_write && has_spsr) {
            // N: Thumb MOV-imm forces 0; otherwise
            // bit 31. V: logical class preserves it; arithmetic
            // replaces it. C: the shifter carry.
            regs.set_cpsr_n(if fx.thumb_mov {
                false
            } else {
                result >> 31 != 0
            });
            regs.set_cpsr_z(result == 0);
            regs.set_cpsr_c(carry);
            if fx.op.replaces_v() {
                regs.set_cpsr_v(overflow);
            }
        }
    }
}

/// Thumb ADD/SUB (register and 3-bit immediate): native micro-op
/// implementation; ADD (op=0) and SUB (op=1) with NZCV writeback.
fn apply_add_sub(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let i = (instr >> 10) & 1 != 0;
    let op = (instr >> 9) & 1 != 0; // 0=ADD, 1=SUB
    let rn_field = ((instr >> 6) & 0x7) as usize;
    let rs = ((instr >> 3) & 0x7) as usize;
    let rd = (instr & 0x7) as usize;

    let rn_val = if i {
        (rn_field & 0x7) as u32
    } else {
        regs.r(rn_field)
    };
    let rs_val = regs.r(rs);
    let result = if op {
        let (r, _) = rs_val.overflowing_sub(rn_val);
        update_nz(regs, r);
        regs.set_cpsr_c(rs_val >= rn_val);
        regs.set_cpsr_v(((rs_val ^ rn_val) & (rs_val ^ r) & 0x80000000) != 0);
        r
    } else {
        let (r, c) = rs_val.overflowing_add(rn_val);
        update_nz(regs, r);
        regs.set_cpsr_c(c);
        regs.set_cpsr_v(((rs_val ^ r) & (rn_val ^ r) & 0x80000000) != 0);
        r
    };
    regs.set_r(rd, result);
    1
}

/// Thumb register ALU (AND/EOR, shifts, ADC/SBC, ROR, TST,
/// NEG/CMP/CMN, ORR/MUL, BIC/MVN): native micro-op implementation
/// with the 1S+mI MUL timing (m from the incoming Rd value).
fn apply_thumb_alu(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let op = ((instr >> 6) & 0xF) as u8;
    let rs = ((instr >> 3) & 0x7) as usize;
    let rd = (instr & 0x7) as usize;
    let rs_val = regs.r(rs);
    let rd_val = regs.r(rd);
    match op {
        0x0 | 0x1 | 0xC..=0xF => thumb_logical(regs, rd, op, rd_val, rs_val),
        0x2..=0x4 | 0x7 => thumb_shift_reg(regs, rd, op, rd_val, rs_val),
        0x5 | 0x6 => thumb_carry_arithmetic(regs, rd, op, rd_val, rs_val),
        0x8 => thumb_test(regs, rd_val, rs_val),
        0x9..=0xB => thumb_compare(regs, rd, op, rd_val, rs_val),
        _ => 1,
    }
}

fn thumb_logical(
    regs: &mut CpuRegisters,
    destination: usize,
    op: u8,
    left: u32,
    right: u32,
) -> u32 {
    let result = match op {
        0x0 => left & right,             // AND
        0x1 => left ^ right,             // EOR
        0xC => left | right,             // ORR
        0xD => left.wrapping_mul(right), // MUL
        0xE => left & !right,            // BIC
        _ => !right,                     // MVN
    };
    regs.set_r(destination, result);
    update_nz(regs, result);
    // GBATEK/ARM ARM: Thumb MUL is 1S+mI like ARM, with m from the
    // incoming Rd value (Rd = Rd*Rs uses Rd for timing); here
    // `left` is the incoming Rd (Rd = Rd*Rs).
    if op == 0xD {
        1 + multiplier_cycles(left)
    } else {
        1
    }
}

fn thumb_shift_reg(
    regs: &mut CpuRegisters,
    destination: usize,
    op: u8,
    value: u32,
    amount: u32,
) -> u32 {
    // Register shift amount zero preserves both the value and carry flag.
    let shift_type = match op {
        0x2 => 0, // LSL
        0x3 => 1, // LSR
        0x4 => 2, // ASR
        _ => 3,   // ROR
    };
    let (result, carry) = barrel_shift_register(value, shift_type, amount, regs.cpsr_c());
    regs.set_r(destination, result);
    update_nz(regs, result);
    regs.set_cpsr_c(carry);
    // GBATEK THUMB cycle table: LSL/LSR/ASR/ROR Rd,Rs costs 1S+1I
    // unconditionally (no zero-amount exception, unlike the ARM-ARM note).
    2
}

fn thumb_carry_arithmetic(
    regs: &mut CpuRegisters,
    destination: usize,
    op: u8,
    left: u32,
    right: u32,
) -> u32 {
    let carry_in = u32::from(regs.cpsr_c());
    let (result, carry, overflow) = if op == 0x5 {
        thumb_add_with_carry(left, right, carry_in)
    } else {
        thumb_subtract_with_carry(left, right, carry_in)
    };
    regs.set_r(destination, result);
    update_nz(regs, result);
    regs.set_cpsr_c(carry);
    regs.set_cpsr_v(overflow);
    1
}

fn thumb_add_with_carry(left: u32, right: u32, carry_in: u32) -> (u32, bool, bool) {
    let sum = u64::from(left) + u64::from(right) + u64::from(carry_in);
    let result = sum as u32;
    let overflow = ((left ^ result) & (right ^ result) & 0x80000000) != 0;
    (result, sum > u64::from(u32::MAX), overflow)
}

fn thumb_subtract_with_carry(left: u32, right: u32, carry_in: u32) -> (u32, bool, bool) {
    let borrow = 1 - carry_in;
    let result = left.wrapping_sub(right).wrapping_sub(borrow);
    let carry = u64::from(left) >= u64::from(right) + u64::from(borrow);
    let overflow = ((left ^ right) & (left ^ result) & 0x80000000) != 0;
    (result, carry, overflow)
}

fn thumb_test(regs: &mut CpuRegisters, left: u32, right: u32) -> u32 {
    // TST has no shifter operand in Thumb, so C is preserved.
    update_nz(regs, left & right);
    1
}

fn thumb_compare(
    regs: &mut CpuRegisters,
    destination: usize,
    op: u8,
    left: u32,
    right: u32,
) -> u32 {
    let (result, carry, overflow) = match op {
        0x9 => (0u32.wrapping_sub(right), right == 0, right == 0x80000000),
        0xA => {
            let result = left.wrapping_sub(right);
            (
                result,
                left >= right,
                ((left ^ right) & (left ^ result) & 0x80000000) != 0,
            )
        }
        _ => {
            let (result, carry) = left.overflowing_add(right);
            (
                result,
                carry,
                ((left ^ result) & (right ^ result) & 0x80000000) != 0,
            )
        }
    };
    if op == 0x9 {
        regs.set_r(destination, result);
    }
    update_nz(regs, result);
    regs.set_cpsr_c(carry);
    regs.set_cpsr_v(overflow);
    1
}

/// Thumb ADD Rd, PC/SP, #imm: native micro-op implementation
/// with word-aligned PC/SP-relative addressing.
fn apply_load_address(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let sp = (instr >> 11) & 1 != 0;
    let rd = ((instr >> 8) & 0x7) as usize;
    let imm = ((instr & 0xFF) as u32) << 2;
    let base = if sp { regs.sp() } else { regs.pc() & !3 };
    regs.set_r(rd, base.wrapping_add(imm));
    1
}

/// Thumb ADD/SUB SP, #imm: native micro-op implementation.
fn apply_sp_offset(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let s = (instr >> 7) & 1 != 0;
    let imm = ((instr & 0x7F) as u32) << 2;
    if s {
        regs.set_sp(regs.sp().wrapping_sub(imm));
    } else {
        regs.set_sp(regs.sp().wrapping_add(imm));
    }
    1
}

/// Thumb hi-register ADD/CMP/MOV (BX never reaches here: the gate
/// routes 0x4700 to `MicroOp::Bx`): native micro-op implementation,
/// including the unreachable BX arm.
fn apply_hi_register(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let op = (instr >> 8) & 0b11;
    let high_destination = (instr >> 7) & 1 != 0;
    let high_source = (instr >> 6) & 1 != 0;
    let rs = ((instr >> 3) & 0x7) as usize + if high_source { 8 } else { 0 };
    let rd = (instr & 0x7) as usize + if high_destination { 8 } else { 0 };
    match op {
        0b00 => {
            // ADD Rd, Rs (a write to PC is a branch: 2S+1N like BX)
            let v = regs.r(rd).wrapping_add(regs.r(rs));
            regs.set_r(rd, v);
            if rd == 15 { 3 } else { 1 }
        }
        0b01 => {
            // CMP Rd, Rs
            let a = regs.r(rd);
            let b = regs.r(rs);
            let (r, _) = a.overflowing_sub(b);
            update_nz(regs, r);
            regs.set_cpsr_c(a >= b);
            regs.set_cpsr_v(((a ^ b) & (a ^ r) & 0x80000000) != 0);
            1
        }
        0b10 => {
            // MOV Rd, Rs (a write to PC is a branch: 2S+1N like BX;
            // Thumb MOV PC does not interwork, unlike BX)
            let v = regs.r(rs);
            regs.set_r(rd, v);
            if rd == 15 { 3 } else { 1 }
        }
        0b11 => {
            // BX Rs
            let target = regs.r(rs);
            let thumb = target & 1 != 0;
            regs.set_cpsr((regs.cpsr() & !(1 << 5)) | ((thumb as u32) << 5));
            regs.set_pc(target & !1);
            3
        }
        _ => 1,
    }
}
/// Execute one queued op; returns its cycle cost.
pub(super) fn apply_op(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    op: MicroOp,
    pc: u32,
    is_thumb: bool,
) -> i64 {
    let mut cycles: i64 = 0;
    match op {
        MicroOp::Internal => cycles += 1,
        MicroOp::CommitAlu(fx) => {
            apply_alu(regs, fx);
            cycles += 1;
        }
        MicroOp::CommitThumb(instr) => {
            apply_commit_thumb(regs, instr);
            cycles += 1;
        }
        MicroOp::CommitDpReg(instr) => {
            // Data-processing performs no bus access: native
            // implementation below; trailing Internals pad the base.
            apply_dp_reg(regs, instr);
            cycles += 1;
        }
        MicroOp::CommitMul(m) => {
            apply_commit_mul(regs, bus, m);
            cycles += 1;
        }
        MicroOp::CommitSwp(instr) => {
            // Atomic read+write inside one commit (bus lock);
            // carry +1, trailing Internals pad the base.
            apply_swp(regs, bus, instr);
            cycles += 1;
        }
        MicroOp::CommitPsr(instr) => {
            apply_psr(regs, instr);
            cycles += 1;
        }
        MicroOp::TakenBranch(branch) => {
            if branch.link {
                regs.set_lr(pc.wrapping_sub(4));
            }
            regs.set_pc(pc.wrapping_add(branch.offset));
            cycles += 1;
        }
        MicroOp::BlHigh(offset) => {
            regs.set_lr(pc.wrapping_add(offset));
            cycles += 1;
        }
        MicroOp::BlLow(offset) => {
            let target = regs.lr().wrapping_add(offset);
            regs.set_lr(pc.wrapping_sub(2) | 1);
            regs.set_pc(target & !1);
            cycles += 1;
        }
        MicroOp::Bx(target) => {
            regs.set_cpsr((regs.cpsr() & !(1 << 5)) | ((target & 1) << 5));
            regs.set_pc(target & !1);
            cycles += 1;
        }
        MicroOp::TrapSwi(swi) => {
            cycles += apply_trap_swi(regs, bus, swi, is_thumb) as i64;
        }
        MicroOp::TrapUnd => {
            cycles += apply_trap_und(regs, is_thumb) as i64;
        }
        MicroOp::MemRead(a) => {
            apply_mem_read(regs, bus, a, is_thumb);
            cycles += 1;
        }
        MicroOp::MemWrite(a) => {
            apply_write(regs, bus, a);
            cycles += 1;
        }
        MicroOp::PcRelRead(r) => {
            apply_pcrel_read(regs, bus, r, is_thumb);
            cycles += 1;
        }
        MicroOp::BlockStart(e) => {
            bus.begin_block_batch(e.is_load, e.fetch_width);
        }
        MicroOp::BlockWord(w) => {
            apply_block_word(regs, bus, w);
            cycles += 1;
        }
        MicroOp::BlockEmpty(e) => {
            apply_block_empty(regs, bus, e);
            cycles += 1;
        }
        MicroOp::BlockEnd(e) => {
            apply_block_end(regs, bus, e);
        }
    }
    cycles
}

/// Single-cycle ALU remainder (plus padded Internals for
/// the multi-cycle forms): native apply per class, which performs
/// no bus access.
fn apply_commit_thumb(regs: &mut CpuRegisters, instr: u16) {
    match instr {
        0x0000..=0x17FF => apply_move_shifted(regs, instr),
        0x1800..=0x1FFF => apply_add_sub(regs, instr),
        0x4000..=0x43FF => apply_thumb_alu(regs, instr),
        0xA000..=0xAFFF => apply_load_address(regs, instr),
        0xB000..=0xB0FF => apply_sp_offset(regs, instr),
        // Gate guarantees hi-reg non-BX here.
        _ => apply_hi_register(regs, instr),
    };
}

/// Thumb LSL/LSR/ASR immediate: native micro-op implementation.
/// Bit-for-bit shift/carry/flag behavior with a 1-cycle base return;
/// the commit op carries the +1 cycle separately, like the other arms here.
fn apply_move_shifted(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let op = (instr >> 11) & 0b11;
    let offset = ((instr >> 6) & 0x1F) as u32;
    let rs = ((instr >> 3) & 0x7) as usize;
    let rd = (instr & 0x7) as usize;
    let rs_val = regs.r(rs);
    let (result, carry) = match op {
        0b00 => {
            // LSL
            if offset == 0 {
                (rs_val, regs.cpsr_c())
            } else {
                let c = (rs_val >> (32 - offset)) & 1 != 0;
                (rs_val << offset, c)
            }
        }
        0b01 => {
            // LSR
            if offset == 0 {
                // LSR #32
                let c = (rs_val >> 31) & 1 != 0;
                (0, c)
            } else {
                let c = (rs_val >> (offset - 1)) & 1 != 0;
                (rs_val >> offset, c)
            }
        }
        0b10 => {
            // ASR
            if offset == 0 {
                let c = (rs_val >> 31) & 1 != 0;
                let v = if c { 0xFFFFFFFF } else { 0 };
                (v, c)
            } else {
                let c = (rs_val >> (offset - 1)) & 1 != 0;
                let v = ((rs_val as i32) >> offset) as u32;
                (v, c)
            }
        }
        _ => (0, false),
    };
    regs.set_r(rd, result);
    regs.set_cpsr_n(result >> 31 != 0);
    regs.set_cpsr_z(result == 0);
    regs.set_cpsr_c(carry);
    1
}

fn apply_commit_mul(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, m: MulEffect) {
    if m.thumb {
        // Mirrors the decoder preamble (fetch break + P-ON
        // erase) plus the ALU handler; m comes from Rd.
        let instr = m.instr as u16;
        let ticks = multiplier_cycles(regs.r((instr & 0x7) as usize));
        bus.charge_fetch_stream_break(0x03000000);
        bus.erase_for_multiply(ticks, 2);
        apply_thumb_alu(regs, instr);
    } else {
        apply_mul(regs, bus, m.instr);
    }
}

/// ARM MUL/MLA/UMULL/UMLAL/SMULL/SMLAL: native micro-op implementation.
/// The multiplier-array carry helpers live in `semantics` (shared leaf
/// logic) and are only called here.
pub(super) fn apply_mul(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    // Multiplies take internal cycles (GBATEK 1S+mI); carried in the base
    // below, plus the fetch-stream break (N32-S32) and the P-ON tick erase.
    let is_long = (instr >> 23) & 1 != 0;
    if is_long {
        // UMULL/UMLAL/SMULL/SMLAL produce an RdHi:RdLo pair.
        return apply_mul_long(regs, bus, instr);
    }
    apply_mul_short(regs, bus, instr)
}

fn apply_mul_short(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let a = (instr >> 21) & 1 != 0; // MLA if 1
    let s = (instr >> 20) & 1 != 0;
    let rd = ((instr >> 16) & 0xF) as usize;
    let rn = ((instr >> 12) & 0xF) as usize;
    let rs = ((instr >> 8) & 0xF) as usize;
    let rm = (instr & 0xF) as usize;

    let rs_val = regs.r(rs);
    let rm_val = regs.r(rm);
    let mut result = rm_val.wrapping_mul(rs_val);
    if a {
        result = result.wrapping_add(regs.r(rn));
    }
    // UNPREDICTABLE Rd=R15 (ARM ARM): never let a multiply hijack the PC
    // and trigger a spurious pipeline refill.
    if rd != 15 {
        regs.set_r(rd, result);
    }

    if s {
        update_nz(regs, result);
    }

    let cycles = multiplier_cycles(rs_val);
    // GBATEK/ARM ARM: MUL=1S+mI, MLA=1S+mI+1I (the 1S is the execute cycle;
    // the opcode fetch is charged separately by the bus). The tick array
    // also breaks the fetch stream and fills prefetch P-ON.
    bus.charge_fetch_stream_break(0x03000000);
    bus.erase_for_multiply(cycles + u32::from(a), 4);
    if a { cycles + 2 } else { cycles + 1 }
}

fn apply_mul_long(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let signed = (instr >> 22) & 1 != 0;
    let accumulate = (instr >> 21) & 1 != 0;
    let set_flags = (instr >> 20) & 1 != 0;
    let rd_hi = ((instr >> 16) & 0xF) as usize;
    let rd_lo = ((instr >> 12) & 0xF) as usize;
    let rs_value = regs.r(((instr >> 8) & 0xF) as usize);
    let rm_value = regs.r((instr & 0xF) as usize);
    let product = multiply_64(rm_value, rs_value, signed);
    let result = if accumulate {
        product.wrapping_add(register_pair(regs, rd_hi, rd_lo))
    } else {
        product
    };
    let hi = (result >> 32) as u32;
    let lo = result as u32;
    // Accumulate seeds must be read before the destination write (the
    // carry model consumes the pre-add RdHi/RdLo, mirroring the HW array).
    let acc_hi = regs.r(rd_hi);
    let acc_lo = regs.r(rd_lo);
    regs.set_r(rd_hi, hi);
    regs.set_r(rd_lo, lo);
    if set_flags {
        regs.set_cpsr_n(hi >> 31 != 0);
        regs.set_cpsr_z(result == 0);
        // N/Z come from the product, but C comes from the Booth array's
        // final carry (see the `multiply` module docs). The array only runs
        // the executed iterations: fully-ticked multiplies use the Hi
        // model, early-out ones the Lo model over the fetched low half.
        let full = multiply_tick_full(rs_value, signed);
        let carry = if full {
            multiply_carry_hi(
                rm_value,
                rs_value,
                if accumulate { acc_hi } else { 0 },
                signed,
            )
        } else {
            multiply_carry_lo(
                rm_value,
                rs_value,
                if accumulate { acc_lo } else { 0 },
                signed,
            )
        };
        regs.set_cpsr_c(carry);
    }
    // GBATEK: UMULL/SMULL=1S+mI+1I, UMLAL/SMLAL=1S+mI+2I.
    let ticks = multiplier_cycles_long(rs_value, signed);
    // Long-MUL post-body breaks the stream; tick erase (xMLAL 2+m, xMULL 1+m).
    bus.charge_fetch_stream_break(0x03000000);
    bus.erase_for_multiply(ticks + 1 + u32::from(accumulate), 4);
    ticks + 2 + u32::from(accumulate)
}

fn apply_mem_read(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, a: MemAccess, is_thumb: bool) {
    // Issue-time sampling: bus access, writeback and break in
    // the issue tick. (A deferred-commit timer re-sample was tried
    // here and FALSIFIED — it breaks 12 hw-test DMA pins that pin
    // issue-time sampling; see the design doc. The queue/drain
    // machinery stays as the verified-neutral execution model.)
    apply_read(regs, bus, a);
    // Thumb single word-load retire hook (feeds the DMA prefetch probe).
    // regs.pc() is the fetch PC here exactly as in step_thumb,
    // so execute-PC adjacency validates the same way.
    if is_thumb && a.width == 4 && !a.signed_load {
        let (addr, _) = resolve_addr(regs, a);
        bus.note_thumb_single_load(regs.pc(), addr);
    }
}

fn apply_pcrel_read(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, r: PcRelRead, is_thumb: bool) {
    // Access order: bus access, then fetch-stream-break.
    regs.set_r(r.rd, bus.read32(r.addr));
    bus.charge_fetch_stream_break(r.addr);
    // Thumb literal retire (loads the marker chain like a load).
    if is_thumb {
        bus.note_thumb_single_load(regs.pc(), r.addr);
    }
}

fn apply_block_word(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, w: BlockWord) {
    // Per-word order: continuation query, then
    // the access. A DMA burst between words resets the address
    // stream (next word N), same as post-DMA CPU accesses.
    let continuation = !w.first && bus.data_continuation_sequential(w.addr);
    bus.set_data_sequential(continuation);
    if w.load {
        apply_block_load(regs, bus, w);
    } else {
        let v = match w.store_value {
            Some(v) => v,
            None => {
                if w.reg == 14 {
                    regs.lr()
                } else {
                    regs.r(w.reg)
                }
            }
        };
        bus.write32(w.addr, v);
    }
}

fn apply_block_load(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, w: BlockWord) {
    let v = bus.read_aligned32(w.addr);
    if w.pc_load {
        regs.set_pc(v);
    } else if w.user_bank {
        regs.set_user_r(w.reg, v);
    } else {
        regs.set_r(w.reg, v);
    }
    // LDM^ exception return (modes with an SPSR bank only);
    // the mode read precedes any change below.
    if w.restore_cpsr && !matches!(regs.cpsr_mode(), 0x10 | 0x1F) {
        regs.set_cpsr(regs.spsr());
    }
}

/// Empty-list block word: the single PC transfer, with the exact
/// bus-call sequence (access, then fetch-stream break; batch framing
/// comes from the surrounding Start/End ops for Thumb, none for ARM).
fn apply_block_empty(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, e: BlockEmptyEffect) {
    if e.reset_sequential {
        bus.set_data_sequential(false);
    }
    if e.load {
        let target = bus.read_aligned32(e.addr);
        if e.break_stream {
            bus.charge_fetch_stream_break(e.addr);
        }
        if let Some((reg, val)) = e.writeback_reg {
            regs.set_r(reg, val);
        }
        regs.set_pc(target);
        // LDM^ loading PC restores CPSR from SPSR. Unlike the
        // `BlockWord` path (which skips USR/SYS), the empty path restores
        // unconditionally.
        if e.restore_cpsr {
            regs.set_cpsr(regs.spsr());
        }
    } else {
        bus.write32(e.addr, e.store_value);
        if e.break_stream {
            bus.charge_fetch_stream_break(e.addr);
        }
        if let Some((reg, val)) = e.writeback_reg {
            regs.set_r(reg, val);
        }
    }
}

fn apply_block_end(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, e: BlockEndEffect) {
    bus.set_data_sequential(false);
    bus.end_block_batch();
    if let Some(sp) = e.sp {
        regs.set_sp(sp);
    }
    if let Some((reg, val)) = e.writeback {
        regs.set_r(reg, val);
    }
    if e.ldm_conflict {
        regs.arm_ldm_conflict();
    }
    bus.charge_fetch_stream_break(e.first_addr);
}
