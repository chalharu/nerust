//! ARM micro-op expansion: one [`MicroOp`] queue per instruction.
//! Pure decode over snapshots (`regs`); shares only the op types
//! from the parent module, so this file depends downward on nothing.
use super::{
    AluEffect, AluImmOp, BlockEmptyEffect, BlockEndEffect, BlockStartEffect, BlockWord,
    BranchEffect, MemAccess, MicroOp, MicroOpVec, MulEffect,
};
use crate::cpu::semantics::{
    barrel_shift, condition_passed, is_psr_transfer, multiplier_cycles, multiplier_cycles_long,
    start_address,
};
use crate::cpu_registers::CpuRegisters;

/// Expand an ARM instruction. `None` = uncovered instruction (no
/// fallback; every documented class expands).
/// `regs` snapshots base pointers and STM store words at queue-fill.
/// Test-only wrapper (the hot path uses [`expand_arm_into`]); kept so
/// the decoder unit tests keep their readable by-value form.
#[cfg(test)]
pub fn expand_arm(instr: u32, regs: &CpuRegisters) -> Option<MicroOpVec> {
    let mut ops = MicroOpVec::new();
    expand_arm_into(instr, regs, &mut ops).map(|()| ops)
}

/// Hot-path expansion: build directly into `out` with no whole-buffer
/// moves (see [`super::expand_thumb::expand_thumb_into`] for why).
/// Fallible leaves truncate `out` to the entry length on `None`.
pub(crate) fn expand_arm_into(instr: u32, regs: &CpuRegisters, out: &mut MicroOpVec) -> Option<()> {
    // B/BL: failed conditions retire in one sequential cycle; taken
    // branches expose both refill cycles independently to the bus.
    if (instr >> 25) & 0x7 == 0b101 {
        let condition = (instr >> 28) as u8;
        if !condition_passed(regs.cpsr(), condition) {
            out.push(MicroOp::Internal);
            return Some(());
        }
        let offset = ((instr & 0x00FF_FFFF) as i32) << 2;
        let offset = (offset << 6) >> 6;
        out.push(MicroOp::Internal);
        out.push(MicroOp::Internal);
        out.push(MicroOp::TakenBranch(BranchEffect {
            offset: offset as u32,
            link: instr & (1 << 24) != 0,
        }));
        return Some(());
    }
    let condition = (instr >> 28) as u8;
    if condition == 0xF {
        // NV: never executes (ARM ARM); it retires as
        // a 1S NOP, so expansion is a single Internal.
        out.push(MicroOp::Internal);
        return Some(());
    }
    // SWI: class 111 with bit 24 set (any condition; the trap
    // number is in bits 23-16 for HLE). Failed conditions retire as
    // [Internal] here (the early return bypasses the wrapper below).
    if (instr >> 25) & 0b111 == 0b111 && (instr >> 24) & 1 == 1 {
        if condition_passed(regs.cpsr(), condition) {
            out.push(MicroOp::TrapSwi(((instr >> 16) & 0xFF) as u8));
        } else {
            out.push(MicroOp::Internal);
        }
        return Some(());
    }
    // UND: coprocessor data class (110) and class 111 without the SWI
    // bit. The GBA has no coprocessor, so all such encodings trap
    // (failed conditions still retire as [Internal]).
    if (instr >> 25) & 0b111 == 0b110 || ((instr >> 25) & 0b111 == 0b111 && (instr >> 24) & 1 == 0)
    {
        if condition_passed(regs.cpsr(), condition) {
            out.push(MicroOp::TrapUnd);
        } else {
            out.push(MicroOp::Internal);
        }
        return Some(());
    }
    let base = out.len();
    // Class pre-dispatch on bits 27-25: the chained attempts below are
    // class-disjoint (each leaf rejects every other class in its first
    // bit tests), so routing straight to the matching leaf skips the
    // failed attempts on the hot path. Relative order inside each route
    // matches the legacy chain exactly.
    match (instr >> 25) & 0b111 {
        // Data-processing immediate (minus MSR-immediate, which the leaf
        // rejects and the PSR leaf takes).
        0b001 => {
            if expand_arm_alu_imm_into(instr, regs, out).is_some() {
                finish_arm_condition(regs, condition, out, base);
                return Some(());
            }
            out.truncate(base);
            if expand_arm_psr_into(instr, out).is_some() {
                finish_arm_condition(regs, condition, out, base);
                return Some(());
            }
            out.truncate(base);
            None
        }
        // Single word/byte transfers (always decode; no other 010/011
        // leaf exists).
        0b010 | 0b011 => {
            if expand_arm_single_into(instr, regs, out).is_some() {
                finish_arm_condition(regs, condition, out, base);
                return Some(());
            }
            out.truncate(base);
            None
        }
        // Block transfers only.
        0b100 => {
            if expand_arm_block_into(instr, regs, out).is_some() {
                finish_arm_condition(regs, condition, out, base);
                return Some(());
            }
            out.truncate(base);
            None
        }
        // Class 000: halfword-tagged forms try the single leaf first
        // (its exclusions route multiply/SWP/PSR/BX onward); untagged
        // forms skip straight to DP-reg (which rejects the tag itself).
        0b000 => {
            if (instr & 0x00000090) == 0x00000090
                && expand_arm_single_into(instr, regs, out).is_some()
            {
                finish_arm_condition(regs, condition, out, base);
                return Some(());
            }
            out.truncate(base);
            if expand_arm_dp_reg_into(instr, regs, out).is_some() {
                finish_arm_condition(regs, condition, out, base);
                return Some(());
            }
            out.truncate(base);
            if expand_arm_mul_into(instr, regs, out).is_some() {
                finish_arm_condition(regs, condition, out, base);
                return Some(());
            }
            out.truncate(base);
            if expand_arm_swp_into(instr, out).is_some() {
                finish_arm_condition(regs, condition, out, base);
                return Some(());
            }
            out.truncate(base);
            if expand_arm_psr_into(instr, out).is_some() {
                finish_arm_condition(regs, condition, out, base);
                return Some(());
            }
            out.truncate(base);
            if expand_arm_bx_into(instr, regs, out).is_some() {
                finish_arm_condition(regs, condition, out, base);
                return Some(());
            }
            out.truncate(base);
            None
        }
        // 101 (branch), 110/111 (UND/SWI) handled above; unreachable.
        _ => None,
    }
}

/// Condition gate for chained ARM attempts: a passed condition keeps
/// the leaf's ops; a failed one replaces them with the 1S NOP retire.
fn finish_arm_condition(regs: &CpuRegisters, condition: u8, out: &mut MicroOpVec, base: usize) {
    if !condition_passed(regs.cpsr(), condition) {
        out.truncate(base);
        out.push(MicroOp::Internal);
    }
}

/// ARM data-processing (register form, I==0): the DP class minus
/// multiply/SWP/PSR/BX/halfword, padded to the pinned base
/// (register-shift +1I, R15-write refill); the commit runs the apply.
/// Test-only wrapper (the hot path uses the `_into` half below).
#[cfg(test)]
pub(crate) fn expand_arm_dp_reg(instr: u32, regs: &CpuRegisters) -> Option<MicroOpVec> {
    let mut ops = MicroOpVec::new();
    expand_arm_dp_reg_into(instr, regs, &mut ops).map(|()| ops)
}

/// Hot-path half of [`expand_arm_dp_reg`].
fn expand_arm_dp_reg_into(instr: u32, regs: &CpuRegisters, out: &mut MicroOpVec) -> Option<()> {
    if (instr >> 26) & 0x3 != 0 || (instr >> 25) & 1 != 0 {
        return None;
    }
    if (instr & 0x0F8000F0) == 0x00800090 || (instr & 0x0FC000F0) == 0x00000090 {
        return None; // Multiply.
    }
    if (instr & 0x0FB00FF0) == 0x01000090 {
        return None; // SWP.
    }
    if is_psr_transfer(instr) {
        return None;
    }
    if (instr & 0x0FFFFFF0) == 0x012FFF10 {
        return None; // BX.
    }
    if (instr & 0x00000090) == 0x00000090 {
        return None; // Halfword / signed transfers.
    }
    let opcode = ((instr >> 21) & 0xF) as u8;
    let rd = ((instr >> 12) & 0xF) as usize;
    let s = (instr >> 20) & 1 != 0;
    let register_shift = (instr >> 4) & 1 != 0;
    let flag_only = matches!(opcode, 0x8..=0xB);
    // USR/SYS have no SPSR (ARM ARM): exception-return restores only
    // apply in modes with an SPSR bank (mode frozen mid-instruction).
    let has_spsr = !matches!(regs.cpsr_mode(), 0x10 | 0x1F);
    // Cycle base, minus the +1 the commit op carries.
    let trailing = if flag_only && rd == 15 && s {
        if has_spsr {
            2 + u32::from(register_shift)
        } else {
            u32::from(register_shift)
        }
    } else {
        u32::from(register_shift) + if rd == 15 && !flag_only { 2 } else { 0 }
    };
    out.push(MicroOp::CommitDpReg(instr));
    out.extend(core::iter::repeat_n(MicroOp::Internal, trailing as usize));
    Some(())
}

/// ARM multiply (short and long): the exact decoder masks, routed
/// before anything else. Expansion is
/// [CommitMul, I..] padded to the pinned base (short MUL 1S+mI,
/// MLA +1I; long 1S+mI+1I, accumulate +1I); the commit runs the native
/// apply, so only the internal-tick split is new.
/// Test-only wrapper (the hot path uses the `_into` half below).
#[cfg(test)]
pub(crate) fn expand_arm_mul(instr: u32, regs: &CpuRegisters) -> Option<MicroOpVec> {
    let mut ops = MicroOpVec::new();
    expand_arm_mul_into(instr, regs, &mut ops).map(|()| ops)
}

/// Hot-path half of [`expand_arm_mul`].
fn expand_arm_mul_into(instr: u32, regs: &CpuRegisters, out: &mut MicroOpVec) -> Option<()> {
    if (instr & 0x0F8000F0) != 0x00800090 && (instr & 0x0FC000F0) != 0x00000090 {
        return None;
    }
    let rs_val = regs.r(((instr >> 8) & 0xF) as usize);
    let trailing = if (instr >> 23) & 1 != 0 {
        // Long: m from the signed/unsigned top-bit rule, +1I, +1I accumulate.
        let signed = (instr >> 22) & 1 != 0;
        let accumulate = (instr >> 21) & 1 != 0;
        multiplier_cycles_long(rs_val, signed) + 1 + u32::from(accumulate)
    } else {
        // Short: m +1I for MLA.
        multiplier_cycles(rs_val) + ((instr >> 21) & 1)
    };
    out.push(MicroOp::CommitMul(MulEffect {
        instr,
        thumb: false,
    }));
    out.extend(core::iter::repeat_n(MicroOp::Internal, trailing as usize));
    Some(())
}

/// ARM SWP/SWPB: the exact decoder mask. Expansion is [CommitSwp,
/// I, I, I] padded to the pinned base (4); the read+write pair stays
/// atomic inside the commit, modeling the HW bus lock.
fn expand_arm_swp_into(instr: u32, out: &mut MicroOpVec) -> Option<()> {
    if (instr & 0x0FB00FF0) != 0x01000090 {
        return None;
    }
    out.push(MicroOp::CommitSwp(instr));
    out.push(MicroOp::Internal);
    out.push(MicroOp::Internal);
    out.push(MicroOp::Internal);
    Some(())
}

/// ARM MRS/MSR: exactly the decoder predicate (all three masks).
/// Single commit op; the handler is pure registers with base 1.
fn expand_arm_psr_into(instr: u32, out: &mut MicroOpVec) -> Option<()> {
    if !is_psr_transfer(instr) {
        return None;
    }
    out.push(MicroOp::CommitPsr(instr));
    Some(())
}

/// ARM BX: the exact decoder mask. Reuses the interworking branch
/// op ([I, I, Bx]); retire flushes with the switched width.
fn expand_arm_bx_into(instr: u32, regs: &CpuRegisters, out: &mut MicroOpVec) -> Option<()> {
    if (instr & 0x0FFFFFF0) != 0x012FFF10 {
        return None;
    }
    out.push(MicroOp::Internal);
    out.push(MicroOp::Internal);
    out.push(MicroOp::Bx(regs.r((instr & 0xF) as usize)));
    Some(())
}

/// Empty-list parameters snapshotted at expansion.
struct BlockEmptySpec {
    base: u32,
    rn: usize,
    pre: bool,
    up: bool,
    s: bool,
    writeback: bool,
    load: bool,
}

/// ARM LDM/STM with an empty register list: the single PC word
/// (GBATEK: Rb+=0x40 address arithmetic, S-bit CPSR restore on LDM^,
/// PC+4 store on STM). No batch framing; the op carries +1 with
/// trailing Internals to the base (LDM+PC 5, STM 2).
fn expand_arm_block_empty_into(spec: BlockEmptySpec, regs: &CpuRegisters, out: &mut MicroOpVec) {
    let addr = start_address(spec.base, 16, spec.pre, spec.up);
    let writeback = spec.writeback.then(|| {
        (
            spec.rn,
            if spec.up {
                spec.base.wrapping_add(0x40)
            } else {
                spec.base.wrapping_sub(0x40)
            },
        )
    });
    out.push(MicroOp::BlockEmpty(BlockEmptyEffect {
        load: spec.load,
        addr,
        writeback_reg: writeback,
        store_value: regs.pc().wrapping_add(4),
        restore_cpsr: spec.s,
        reset_sequential: false,
        // Standalone (no BlockEnd follows): break here.
        break_stream: true,
    }));
    out.extend(core::iter::repeat_n(
        MicroOp::Internal,
        if spec.load { 4 } else { 1 },
    ));
}

/// STM store-word snapshot at expansion (frozen registers and mode):
/// user-bank reads, the stored-base quirk, and r15 as instruction+12.
fn block_store_value(
    load: bool,
    stored_base: Option<(usize, u32)>,
    user_bank: bool,
    regs: &CpuRegisters,
    reg: usize,
) -> Option<u32> {
    if load {
        return None;
    }
    let base_val = match stored_base {
        Some((base_register, value)) if base_register == reg => value,
        _ => {
            if user_bank {
                regs.user_r(reg)
            } else {
                regs.r(reg)
            }
        }
    };
    Some(base_val.wrapping_add(if reg == 15 { 4 } else { 0 }))
}

/// Writeback is not allowed if base in list and L==1 (UNPREDICTABLE).
fn block_writeback(
    writeback_flag: bool,
    load: bool,
    list: u32,
    rn: usize,
    final_addr: u32,
) -> Option<(usize, u32)> {
    if writeback_flag && !(load && (list >> rn) & 1 != 0) {
        Some((rn, final_addr))
    } else {
        None
    }
}

/// Pad the pinned `transfer_cycles` base (LDM 2+n, LDM+PC 4+n,
/// STM 1+n; words already carry +1 each).
fn block_trailing(load: bool, list: u32) -> usize {
    if load {
        if list & (1 << 15) != 0 { 4 } else { 2 }
    } else {
        1
    }
}

/// ARM LDM/STM, non-empty lists including the S bit (user-bank
/// transfers and CPSR-restoring exception returns). P/U address
/// modes, writeback (skipped for the UNPREDICTABLE load-with-base-
/// in-list), the STM stored-base quirk, PC loads (retire flushes),
/// and the empty-list single-PC-word transfer are covered.
fn expand_arm_block_into(instr: u32, regs: &CpuRegisters, out: &mut MicroOpVec) -> Option<()> {
    if (instr >> 25) & 0x7 != 0b100 {
        return None;
    }
    let pre = (instr >> 24) & 1 != 0;
    let up = (instr >> 23) & 1 != 0;
    let s = (instr >> 22) & 1 != 0;
    let writeback_flag = (instr >> 21) & 1 != 0;
    let load = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let list = instr & 0xFFFF;
    let base = regs.r(rn);
    if list == 0 {
        expand_arm_block_empty_into(
            BlockEmptySpec {
                base,
                rn,
                pre,
                up,
                s,
                writeback: writeback_flag,
                load,
            },
            regs,
            out,
        );
        return Some(());
    }
    let count = list.count_ones();
    let start = start_address(base, count, pre, up);
    // Without writeback the stored base uses the OLD value; only W=1
    // stores the NEW value for non-first occurrences.
    let final_addr = if up {
        base.wrapping_add(count * 4)
    } else {
        base.wrapping_sub(count * 4)
    };
    let stored_base =
        (writeback_flag && !load && list & (1 << rn) != 0 && rn != list.trailing_zeros() as usize)
            .then_some((rn, final_addr));
    // User-bank selection (S without a PC load); snapshot at expansion
    // (mode frozen until a PC load, which clears this flag).
    let user_bank = s && !(load && list & (1 << 15) != 0);
    out.push(MicroOp::BlockStart(BlockStartEffect {
        is_load: load,
        fetch_width: 4,
    }));
    for (i, reg) in (0..16usize)
        .filter(|reg| list & (1 << reg) != 0)
        .enumerate()
    {
        let store_value = block_store_value(load, stored_base, user_bank, regs, reg);
        out.push(MicroOp::BlockWord(BlockWord {
            addr: start.wrapping_add(i as u32 * 4),
            reg,
            load,
            first: i == 0,
            pc_load: load && reg == 15,
            user_bank: load && user_bank,
            restore_cpsr: load && s && reg == 15,
            store_value,
        }));
    }
    // Writeback not allowed if base in list and L==1 (UNPREDICTABLE).
    let writeback = block_writeback(writeback_flag, load, list, rn, final_addr);
    // Post-LDM^ conflict, armed at expansion (the PC
    // case never sets user_bank, so expansion-time mode is exact).
    let conflict = load && user_bank && !matches!(regs.cpsr_mode(), 0x10 | 0x1F);
    out.push(MicroOp::BlockEnd(BlockEndEffect {
        sp: None,
        writeback,
        ldm_conflict: conflict,
        first_addr: start,
    }));
    // Pad the pinned `transfer_cycles` base (LDM 2+n, LDM+PC 4+n,
    // STM 1+n; words already carry +1 each).
    let trailing = block_trailing(load, list);
    out.extend(core::iter::repeat_n(MicroOp::Internal, trailing));
    Some(())
}

/// ARM data-processing immediate, no R15 dest, no S+rotate (see gate
/// above). Rn==15 (PC) reads are covered: the effect resolves Rn live
/// at execution, matching the handler's execute-stage read (PC still
/// leads by 8; advance lands at retire).
fn expand_arm_alu_imm_into(instr: u32, regs: &CpuRegisters, out: &mut MicroOpVec) -> Option<()> {
    // Data-processing class (bits27-26 == 00), immediate form (I==1).
    // With I==1 the decoder can only route to DP or PSR-immediate
    // (multiply/SWP/BX/halfword all require I==0); the MSR-immediate
    // mask keeps the latter on the PSR branch. Bit4 is plain imm12.
    if (instr >> 26) & 0x3 != 0 || (instr >> 25) & 1 != 1 {
        return None;
    }
    if (instr & 0x0FB0F000) == 0x0320F000 {
        return None; // MSR-immediate.
    }
    let opcode = ((instr >> 21) & 0xF) as u8;
    let op = match opcode {
        0x0 => AluImmOp::And,
        0x1 => AluImmOp::Eor,
        0x2 => AluImmOp::Sub,
        0x3 => AluImmOp::Rsb,
        0x4 => AluImmOp::Add,
        0x5 => AluImmOp::Adc,
        0x6 => AluImmOp::Sbc,
        0x7 => AluImmOp::Rsc,
        0x8 => AluImmOp::Tst,
        0x9 => AluImmOp::Teq,
        0xA => AluImmOp::Cmp,
        0xB => AluImmOp::Cmn,
        0xC => AluImmOp::Orr,
        0xD => AluImmOp::Mov,
        0xE => AluImmOp::Bic,
        _ => AluImmOp::Mvn,
    };
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let set_flags = instr >> 20 & 1 == 1;
    let rot = ((instr >> 8) & 0xF) * 2;
    let imm = (instr & 0xFF).rotate_right(rot);
    // Shifter carry-out for S with a rotated immediate; otherwise the
    // live CPSR carry (rot == 0) or flag-neutral (without S).
    let carry = if rot != 0 && set_flags {
        Some((instr & 0xFF) & (1 << (rot - 1)) != 0)
    } else {
        None
    };
    // S with Rd==15 is an exception return (or flags-only form);
    // USR/SYS have no SPSR (mode frozen mid-instruction).
    let has_spsr = !matches!(regs.cpsr_mode(), 0x10 | 0x1F);
    // Cycle base minus the commit's +1: flag-only Rd15 restores (3)
    // or updates flags (1); Rd15 writes refill (+2).
    let trailing = if op.is_flag_only() && rd == 15 && set_flags {
        if has_spsr { 2 } else { 0 }
    } else if rd == 15 && !op.is_flag_only() {
        2
    } else {
        0
    };
    out.push(MicroOp::CommitAlu(AluEffect {
        op,
        rd,
        rn,
        imm,
        set_flags,
        thumb_mov: false,
        carry,
    }));
    out.extend(core::iter::repeat_n(MicroOp::Internal, trailing as usize));
    Some(())
}

/// ARM LDR/STR word/byte (immediate and register offset) and
/// LDRH/STRH unsigned-immediate, including R15 base/dest. Register
/// offsets snapshot the barrel-shifted Rm at expansion (frozen
/// registers); STR of R15 snapshots instruction+12. Loads expand to
/// [Read, I, I] (= 3; +2 more for R15 loads) and stores to [Write, I]
/// (= 2) per GBATEK.
/// Test-only wrapper (the hot path uses the `_into` half below).
#[cfg(test)]
pub(crate) fn expand_arm_single(instr: u32, regs: &CpuRegisters) -> Option<MicroOpVec> {
    let mut ops = MicroOpVec::new();
    expand_arm_single_into(instr, regs, &mut ops).map(|()| ops)
}

/// Hot-path half of [`expand_arm_single`].
fn expand_arm_single_into(instr: u32, regs: &CpuRegisters, out: &mut MicroOpVec) -> Option<()> {
    // Class predicates first: the snapshot below costs a register
    // read, so failed attempts (the common case in the chain) must
    // not pay for it.
    // Word/byte class (bits27-26 == 01), immediate or register offset.
    let is_word = (instr >> 26) & 0x3 == 0b01;
    // Halfword class (bits27-25 == 000, bit7+bit4 set): immediate and
    // register offsets, all S:H shapes (unsigned half, signed byte /
    // half; S:H == 00 behaves as halfword).
    // Multiply/SWP/PSR/BX patterns carry the tag too, so the same
    // decoder exclusions apply (decode tests them first).
    let is_half = !is_word && (instr >> 25) & 0x7 == 0 && (instr & 0x00000090) == 0x00000090;
    if !is_word && !is_half {
        return None;
    }
    let dec = SingleDecoded {
        l: (instr >> 20) & 1 == 1,
        pre_indexed: (instr >> 24) & 1 == 1,
        writeback: (instr >> 24) & 1 == 0 || (instr >> 21) & 1 == 1,
        rn: ((instr >> 16) & 0xF) as usize,
        rd: ((instr >> 12) & 0xF) as usize,
        subtract: (instr >> 23) & 1 == 0,
        // STR of R15 stores instruction+12.
        store_value: single_store_value(instr, regs),
    };
    if is_word {
        single_word_into(instr, regs, &dec, out);
        return Some(());
    }
    single_half_into(instr, regs, &dec, out)
}

/// Shared decode for both single-transfer classes.
struct SingleDecoded {
    l: bool,
    pre_indexed: bool,
    writeback: bool,
    rn: usize,
    rd: usize,
    subtract: bool,
    store_value: Option<u32>,
}

#[inline]
fn single_store_value(instr: u32, regs: &CpuRegisters) -> Option<u32> {
    let l = (instr >> 20) & 1 == 1;
    let rd = ((instr >> 12) & 0xF) as usize;
    if !l && rd == 15 {
        Some(regs.r(15).wrapping_add(4))
    } else {
        None
    }
}

/// Pad the pinned base: loads 3 (5 for R15), stores 2.
/// Tiny (single call chain depth matters: 46% of ARM instructions);
/// forced-inline into both callers.
#[inline]
fn single_ops_into(l: bool, rd: usize, acc: MemAccess, out: &mut MicroOpVec) {
    if l {
        out.push(MicroOp::MemRead(acc));
        out.push(MicroOp::Internal);
        out.push(MicroOp::Internal);
        if rd == 15 {
            out.push(MicroOp::Internal);
            out.push(MicroOp::Internal);
        }
    } else {
        out.push(MicroOp::MemWrite(acc));
        out.push(MicroOp::Internal);
    }
}

fn single_word_into(instr: u32, regs: &CpuRegisters, dec: &SingleDecoded, out: &mut MicroOpVec) {
    let offset = if (instr >> 25) & 1 != 0 {
        let rm_val = regs.r((instr & 0xF) as usize);
        let (shifted, _) = barrel_shift(
            rm_val,
            ((instr >> 5) & 0b11) as u8,
            (instr >> 7) & 0x1F,
            regs.cpsr_c(),
        );
        shifted
    } else {
        instr & 0xFFF
    };
    let acc = MemAccess {
        width: if instr & (1 << 22) != 0 { 1 } else { 4 },
        rd: dec.rd,
        rn: dec.rn,
        offset,
        offset_register: None,
        subtract: dec.subtract,
        is_sp: false,
        signed_load: false,
        post_indexed: !dec.pre_indexed,
        writeback: dec.writeback,
        store_value: dec.store_value,
        halfword_odd_quirk: false,
    };
    // Bus calls and order match the apply step, so totals agree by construction.
    single_ops_into(dec.l, dec.rd, acc, out);
}

fn single_half_into(
    instr: u32,
    regs: &CpuRegisters,
    dec: &SingleDecoded,
    out: &mut MicroOpVec,
) -> Option<()> {
    if (instr & 0x0F8000F0) == 0x00800090 || (instr & 0x0FC000F0) == 0x00000090 {
        return None; // Multiply.
    }
    if (instr & 0x0FB00FF0) == 0x01000090 {
        return None; // SWP.
    }
    if is_psr_transfer(instr) {
        return None;
    }
    if (instr & 0x0FFFFFF0) == 0x012FFF10 {
        return None; // BX.
    }
    let signed = (instr >> 6) & 1 != 0;
    let half = (instr >> 5) & 1 != 0;
    let offset = if (instr >> 22) & 1 != 0 {
        (((instr >> 8) & 0xF) << 4) | (instr & 0xF)
    } else {
        regs.r((instr & 0xF) as usize)
    };
    let acc = MemAccess {
        width: if signed && !half { 1 } else { 2 },
        rd: dec.rd,
        rn: dec.rn,
        offset,
        offset_register: None,
        subtract: dec.subtract,
        is_sp: false,
        signed_load: signed,
        post_indexed: !dec.pre_indexed,
        writeback: dec.writeback,
        store_value: dec.store_value,
        halfword_odd_quirk: false,
    };
    single_ops_into(dec.l, dec.rd, acc, out);
    Some(())
}
