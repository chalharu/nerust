//! Micro-op expansion for covered instructions, interpreted one step at a time.
//! Decode stays instruction-atomic; uncovered instructions return `None` (legacy path).
//! Totals match the legacy step per class, pinned by the differential tests below.

use std::collections::VecDeque;

use crate::cpu::arm::{handle_swp as swp_handle, is_psr_transfer};
use crate::cpu::arm_opcodes::block_transfer::start_address;
use crate::cpu::arm_opcodes::data_processing::handle as dp_handle;
use crate::cpu::arm_opcodes::helpers::{barrel_shift, condition_passed};
use crate::cpu::arm_opcodes::multiply::{
    handle as mul_handle, multiplier_cycles, multiplier_cycles_long,
};
use crate::cpu::arm_opcodes::psr_transfer::handle as psr_handle;
use crate::cpu::thumb_opcodes::alu::{
    handle as thumb_alu_handle, handle_load_address, handle_sp_offset,
};
use crate::cpu::thumb_opcodes::{add_sub, hi_register, move_shifted};
use crate::cpu_pipeline::fill_pipeline;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// One sub-instruction effect; effects land at execute-stage points with
/// the legacy bus-call order (access, then fetch-stream-break).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicroOp {
    Internal,
    CommitAlu(AluEffect),
    /// ARM data-processing (register form): raw word, applied by the
    /// legacy handler with its base returned separately (trailing
    /// `Internal`s pad it). Costs +1 here; the legacy path performs no
    /// bus access, so only the cycle split is new.
    CommitDpReg(u32),
    /// Multiply commit: raw word plus mode. ARM short/long delegate to
    /// the legacy handler; Thumb MUL mirrors the decoder preamble
    /// (fetch break + P-ON erase) plus the ALU handler. Costs +1; the
    /// m internal ticks become trailing `Internal` event points.
    CommitMul(MulEffect),
    /// SWP commit: raw word, applied atomically by the legacy handler
    /// (the HW bus lock forbids DMA between the read and the write, so
    /// the pair never splits across ops). Costs +1; the remaining base
    /// becomes trailing `Internal` event points.
    CommitSwp(u32),
    /// ARM MRS/MSR: raw word, applied by the legacy handler (pure
    /// registers, no bus; T-bit preserving, so no mode-switch flush).
    /// Single op carrying the whole base of 1.
    CommitPsr(u32),
    /// Thumb ALU remainder (move-shifted, ADD/SUB, reg-ALU, hi-reg):
    /// raw halfword; the apply step delegates to the legacy handler
    /// for the matching class. Costs +1; multi-cycle forms (reg
    /// shifts, hi-reg PC writes) pad trailing `Internal`s.
    CommitThumb(u16),
    /// Long-BL high half: sign-extended (offset11<<12); writes LR.
    BlHigh(u32),
    /// Long-BL low half: (offset11<<1); target=LR+offset,
    /// LR=(pc-2)|1, branches (retire flushes).
    BlLow(u32),
    /// BX target snapshotted at expansion (mode bit included);
    /// switches T and branches (retire flushes).
    Bx(u32),
    TakenBranch(BranchEffect),
    MemRead(MemAccess),
    MemWrite(MemAccess),
    /// Thumb LDR-literal: pool address snapshotted at expansion
    /// (`(pc & !3) + imm`, pc frozen pre-instruction like the legacy
    /// handler's execute-stage read).
    PcRelRead(PcRelRead),
    /// Open the block batch (`begin_block_batch(is_load, fetch_width)`).
    /// Zero-cost structural op.
    BlockStart(BlockStartEffect),
    BlockWord(BlockWord),
    /// Close the batch, land end-commits, single fetch-stream break.
    /// Zero-cost structural op; trailing `Internal`s pad the base.
    BlockEnd(BlockEndEffect),
}

/// Block-batch opener: `is_load` selects the GBALoad/GBAStore word
/// convention, `fetch_width` (4 ARM, 2 Thumb) the erase-floor N.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockStartEffect {
    pub is_load: bool,
    pub fetch_width: u8,
}
/// Instruction-end commit for a block transfer. `sp` goes through
/// `set_sp` exactly like the legacy PUSH/POP path (NOT `set_r`, which
/// also spills to the user bank inside the LDM^ conflict window);
/// `writeback` is the LDM/STM base update via `set_r`; `ldm_conflict`
/// arms the post-LDM^ bank-conflict window (ARM S-bit loads to the
/// user bank outside USR/SYS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockEndEffect {
    pub sp: Option<u32>,
    pub writeback: Option<(usize, u32)>,
    pub ldm_conflict: bool,
}

/// Thumb LDR (literal): word-aligned pool address plus dest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcRelRead {
    pub addr: u32,
    pub rd: usize,
}

/// One block word (PUSH/POP, LDM/STM). `addr` is snapshotted at expansion (queue-fill
/// runs on pre-instruction state); values are read at execution, which
/// matches the legacy loop because no CPU register changes between the
/// words of one instruction (bus ticks never touch registers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockWord {
    pub addr: u32,
    /// Source/dest low register, or 14 for the PUSH LR slot.
    pub reg: usize,
    pub load: bool,
    pub first: bool,
    /// POP {..,PC} final word: `set_pc` (retire flushes the pipeline).
    pub pc_load: bool,
    /// ARM LDM^ (S bit, no PC in list): loads land in the user bank.
    /// Snapshot at expansion (mode frozen until a PC load, which is
    /// always last and never takes this path).
    pub user_bank: bool,
    /// ARM LDM^ loading PC: restore CPSR from SPSR (skipped in
    /// USR/SYS, which have none). Evaluated at execution, before any
    /// mode change within this instruction.
    pub restore_cpsr: bool,
    /// Precomputed store word (STM base-in-list quirk: non-first
    /// occurrences of the base store the final address). Snapshot at
    /// expansion; registers are frozen across the words of one
    /// instruction, so this matches the legacy in-loop evaluation.
    pub store_value: Option<u32>,
}

/// Multiply commit descriptor: raw word (`thumb` form in low 16 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MulEffect {
    pub instr: u32,
    pub thumb: bool,
}

/// Final execute-stage effect of a direct branch. The two preceding
/// `Internal` ops model pipeline refill cycles; BL writes LR only when
/// this final op executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BranchEffect {
    pub offset: u32,
    pub link: bool,
}

/// One data access. `addr = base +/- offset` is resolved at interpret
/// time from live registers (matches handler evaluation order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemAccess {
    pub width: u8,
    pub rd: usize,
    pub rn: usize,
    pub offset: u32,
    pub offset_register: Option<usize>,
    pub subtract: bool,
    pub is_sp: bool,
    pub signed_load: bool,
    pub post_indexed: bool,
    pub writeback: bool,
    /// Thumb LDRSH odd-address bus quirk (halfword read + ROM merge +
    /// high-byte sign). ARM LDRSH at odd addresses is a plain
    /// sign-extended byte read like the legacy halfword handler.
    pub halfword_odd_quirk: bool,
    /// Precomputed store word (ARM STR of R15: instruction+12).
    /// Snapshot at expansion; registers are frozen across one
    /// instruction, matching the legacy in-handler evaluation.
    pub store_value: Option<u32>,
}

/// Register effect of an ALU-immediate instruction, fully decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AluEffect {
    pub op: AluImmOp,
    pub rd: usize,
    pub rn: usize,
    pub imm: u32,
    pub set_flags: bool,
    /// Thumb MOV-imm forces N=0 (legacy `handle_imm` quirk); ARM MOVS
    /// takes N from bit 31. V is preserved by MOV in both modes.
    pub thumb_mov: bool,
    /// Shifter carry-out for S with a rotated immediate
    /// (bit(rot-1) of the imm8). None selects the live CPSR carry
    /// (rot == 0, or flag-neutral without S).
    pub carry: Option<bool>,
}

/// Covered ALU-immediate operations (full ARM DP-imm set; Thumb uses
/// Mov/Cmp/Add/Sub).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AluImmOp {
    Mov,
    Add,
    Sub,
    Cmp,
    And,
    Eor,
    Rsb,
    Adc,
    Sbc,
    Rsc,
    Tst,
    Teq,
    Cmn,
    Orr,
    Bic,
    Mvn,
}

impl AluImmOp {
    /// TST/TEQ/CMP/CMN update flags without writing Rd.
    fn is_flag_only(self) -> bool {
        matches!(
            self,
            AluImmOp::Tst | AluImmOp::Teq | AluImmOp::Cmp | AluImmOp::Cmn
        )
    }

    /// Logical operations preserve V; arithmetic operations replace it
    /// (legacy `update_flags` rule).
    fn replaces_v(self) -> bool {
        matches!(
            self,
            AluImmOp::Add
                | AluImmOp::Sub
                | AluImmOp::Rsb
                | AluImmOp::Adc
                | AluImmOp::Sbc
                | AluImmOp::Rsc
                | AluImmOp::Cmp
                | AluImmOp::Cmn
        )
    }
}

/// Expand an ARM instruction. `None` = not covered yet (legacy path).
/// `regs` snapshots base pointers and STM store words at queue-fill.
pub fn expand_arm(instr: u32, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    // B/BL: failed conditions retire in one sequential cycle; taken
    // branches expose both refill cycles independently to the bus.
    if (instr >> 25) & 0x7 == 0b101 {
        let condition = (instr >> 28) as u8;
        if !condition_passed(regs.cpsr(), condition) {
            return Some(vec![MicroOp::Internal]);
        }
        let offset = ((instr & 0x00FF_FFFF) as i32) << 2;
        let offset = (offset << 6) >> 6;
        return Some(vec![
            MicroOp::Internal,
            MicroOp::Internal,
            MicroOp::TakenBranch(BranchEffect {
                offset: offset as u32,
                link: instr & (1 << 24) != 0,
            }),
        ]);
    }
    let condition = (instr >> 28) as u8;
    if condition == 0xF {
        return None;
    }
    let ops = expand_arm_alu_imm(instr, regs)
        .or_else(|| expand_arm_single(instr, regs))
        .or_else(|| expand_arm_block(instr, regs))
        .or_else(|| expand_arm_dp_reg(instr, regs))
        .or_else(|| expand_arm_mul(instr, regs))
        .or_else(|| expand_arm_swp(instr))
        .or_else(|| expand_arm_psr(instr))
        .or_else(|| expand_arm_bx(instr, regs))?;
    Some(if condition_passed(regs.cpsr(), condition) {
        ops
    } else {
        vec![MicroOp::Internal]
    })
}

/// ARM data-processing (register form, I==0): the DP class minus
/// multiply/SWP/PSR/BX/halfword, padded to the legacy base; semantics
/// match by construction (commit delegates to the legacy handler).
fn expand_arm_dp_reg(instr: u32, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
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
    // Legacy `handle` base, minus the +1 the commit op carries.
    let trailing = if flag_only && rd == 15 && s {
        if has_spsr {
            2 + u32::from(register_shift)
        } else {
            u32::from(register_shift)
        }
    } else {
        u32::from(register_shift) + if rd == 15 && !flag_only { 2 } else { 0 }
    };
    let mut ops = vec![MicroOp::CommitDpReg(instr)];
    ops.extend(vec![MicroOp::Internal; trailing as usize]);
    Some(ops)
}

/// ARM multiply (short and long): the exact decoder masks, which route
/// to `multiply::handle` before anything else. Expansion is
/// [CommitMul, I..] padded to the legacy base (short MUL 1S+mI,
/// MLA +1I; long 1S+mI+1I, accumulate +1I); the commit delegates to
/// the legacy handler, so only the internal-tick split is new.
fn expand_arm_mul(instr: u32, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
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
    let mut ops = vec![MicroOp::CommitMul(MulEffect {
        instr,
        thumb: false,
    })];
    ops.extend(vec![MicroOp::Internal; trailing as usize]);
    Some(ops)
}

/// ARM SWP/SWPB: the exact decoder mask. Expansion is [CommitSwp,
/// I, I, I] padded to the legacy base (4); the read+write pair stays
/// atomic inside the commit, modeling the HW bus lock.
fn expand_arm_swp(instr: u32) -> Option<Vec<MicroOp>> {
    if (instr & 0x0FB00FF0) != 0x01000090 {
        return None;
    }
    Some(vec![
        MicroOp::CommitSwp(instr),
        MicroOp::Internal,
        MicroOp::Internal,
        MicroOp::Internal,
    ])
}

/// ARM MRS/MSR: exactly the decoder predicate (all three masks).
/// Single commit op; the handler is pure registers with base 1.
fn expand_arm_psr(instr: u32) -> Option<Vec<MicroOp>> {
    if !is_psr_transfer(instr) {
        return None;
    }
    Some(vec![MicroOp::CommitPsr(instr)])
}

/// ARM BX: the exact decoder mask. Reuses the interworking branch
/// op ([I, I, Bx]); retire flushes with the switched width.
fn expand_arm_bx(instr: u32, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    if (instr & 0x0FFFFFF0) != 0x012FFF10 {
        return None;
    }
    Some(vec![
        MicroOp::Internal,
        MicroOp::Internal,
        MicroOp::Bx(regs.r((instr & 0xF) as usize)),
    ])
}

/// ARM LDM/STM, non-empty lists including the S bit (user-bank
/// transfers and CPSR-restoring exception returns). P/U address
/// modes, writeback (skipped for the UNPREDICTABLE load-with-base-
/// in-list), the STM stored-base quirk, and PC loads (retire
/// flushes) are covered; the empty-list transfer stays legacy.
fn expand_arm_block(instr: u32, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
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
    if list == 0 {
        return None;
    }
    let base = regs.r(rn);
    let count = list.count_ones();
    let slots: Vec<usize> = (0..16).filter(|i| list & (1 << i) != 0).collect();
    let start = start_address(base, count, pre, up);
    // Without writeback the stored base uses the OLD value; only W=1
    // stores the NEW value for non-first occurrences (legacy logic).
    let final_addr = if up {
        base.wrapping_add(count * 4)
    } else {
        base.wrapping_sub(count * 4)
    };
    let stored_base =
        (writeback_flag && !load && list & (1 << rn) != 0 && rn != list.trailing_zeros() as usize)
            .then_some((rn, final_addr));
    // User-bank selection (S without a PC load); snapshot at expansion
    // like the legacy pre-loop computation (mode frozen until a PC
    // load, which clears this flag).
    let user_bank = s && !(load && list & (1 << 15) != 0);
    let mut ops = vec![MicroOp::BlockStart(BlockStartEffect {
        is_load: load,
        fetch_width: 4,
    })];
    for (i, reg) in slots.iter().enumerate() {
        // STM store words snapshot at expansion (frozen registers and
        // mode): user-bank reads, the stored-base quirk, and r15 as
        // instruction+12 (legacy `store_register`).
        let store_value = if load {
            None
        } else {
            let base_val = stored_base
                .filter(|(base_register, _)| *base_register == *reg)
                .map_or_else(
                    || {
                        if user_bank {
                            regs.user_r(*reg)
                        } else {
                            regs.r(*reg)
                        }
                    },
                    |(_, value)| value,
                );
            Some(base_val.wrapping_add(if *reg == 15 { 4 } else { 0 }))
        };
        ops.push(MicroOp::BlockWord(BlockWord {
            addr: start.wrapping_add(i as u32 * 4),
            reg: *reg,
            load,
            first: i == 0,
            pc_load: load && *reg == 15,
            user_bank: load && user_bank,
            restore_cpsr: load && s && *reg == 15,
            store_value,
        }));
    }
    // Writeback not allowed if base in list and L==1 (UNPREDICTABLE).
    let writeback = if writeback_flag && !(load && (list >> rn) & 1 != 0) {
        Some((rn, final_addr))
    } else {
        None
    };
    // Post-LDM^ conflict, armed exactly like the legacy tail (the PC
    // case never sets user_bank, so expansion-time mode is exact).
    let conflict = load && user_bank && !matches!(regs.cpsr_mode(), 0x10 | 0x1F);
    ops.push(MicroOp::BlockEnd(BlockEndEffect {
        sp: None,
        writeback,
        ldm_conflict: conflict,
    }));
    // Pad the legacy `transfer_cycles` base (LDM 2+n, LDM+PC 4+n,
    // STM 1+n; words already carry +1 each).
    let trailing = if load {
        if list & (1 << 15) != 0 { 4 } else { 2 }
    } else {
        1
    };
    ops.extend(vec![MicroOp::Internal; trailing as usize]);
    Some(ops)
}

/// ARM data-processing immediate, no R15 dest, no S+rotate (see gate
/// above). Rn==15 (PC) reads are covered: the effect resolves Rn live
/// at execution, matching the handler's execute-stage read (PC still
/// leads by 8; advance lands at retire).
fn expand_arm_alu_imm(instr: u32, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
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
    // Legacy base minus the commit's +1: flag-only Rd15 restores (3)
    // or updates flags (1); Rd15 writes refill (+2).
    let trailing = if op.is_flag_only() && rd == 15 && set_flags {
        if has_spsr { 2 } else { 0 }
    } else if rd == 15 && !op.is_flag_only() {
        2
    } else {
        0
    };
    let mut ops = vec![MicroOp::CommitAlu(AluEffect {
        op,
        rd,
        rn,
        imm,
        set_flags,
        thumb_mov: false,
        carry,
    })];
    ops.extend(vec![MicroOp::Internal; trailing as usize]);
    Some(ops)
}

/// ARM LDR/STR word/byte (immediate and register offset) and
/// LDRH/STRH unsigned-immediate, including R15 base/dest. Register
/// offsets snapshot the barrel-shifted Rm at expansion (frozen
/// registers); STR of R15 snapshots instruction+12. Loads expand to
/// [Read, I, I] (= 3; +2 more for R15 loads) and stores to [Write, I]
/// (= 2), matching `single_transfer`/`halfword_transfer`.
fn expand_arm_single(instr: u32, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    let l = (instr >> 20) & 1 == 1;
    let pre_indexed = (instr >> 24) & 1 == 1;
    let writeback = !pre_indexed || (instr >> 21) & 1 == 1;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let subtract = (instr >> 23) & 1 == 0;
    // STR of R15 stores instruction+12 (legacy `single_transfer`).
    let store_value = if !l && rd == 15 {
        Some(regs.r(15).wrapping_add(4))
    } else {
        None
    };
    // Pad the legacy base: loads 3 (5 for R15), stores 2.
    let ops = |acc: MemAccess| {
        if l {
            let mut ops = vec![MicroOp::MemRead(acc), MicroOp::Internal, MicroOp::Internal];
            if rd == 15 {
                ops.extend([MicroOp::Internal, MicroOp::Internal]);
            }
            ops
        } else {
            vec![MicroOp::MemWrite(acc), MicroOp::Internal]
        }
    };
    // Word/byte class (bits27-26 == 01), immediate or register offset.
    if (instr >> 26) & 0x3 == 0b01 {
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
            rd,
            rn,
            offset,
            offset_register: None,
            subtract,
            is_sp: false,
            signed_load: false,
            post_indexed: !pre_indexed,
            writeback,
            store_value,
            halfword_odd_quirk: false,
        };
        // Same bus calls and order as legacy, so totals agree by construction.
        return Some(ops(acc));
    }
    // Halfword class (bits27-25 == 000, bit7+bit4 set): immediate and
    // register offsets, all S:H shapes (unsigned half, signed byte /
    // half; S:H == 00 behaves as halfword like the legacy handler).
    // Multiply/SWP/PSR/BX patterns carry the tag too, so the decoder
    // exclusions are mirrored (decode tests them first).
    if (instr >> 25) & 0x7 == 0 && (instr & 0x00000090) == 0x00000090 {
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
            rd,
            rn,
            offset,
            offset_register: None,
            subtract,
            is_sp: false,
            signed_load: signed,
            post_indexed: !pre_indexed,
            writeback,
            store_value,
            halfword_odd_quirk: false,
        };
        return Some(ops(acc));
    }
    None
}

/// Expand a Thumb instruction. `None` = not covered yet (legacy path).
/// `regs` snapshots stack/base pointers and STM store words at
/// queue-fill (pre-instruction state; registers are frozen across the
/// words of one instruction).
pub fn expand_thumb(instr: u16, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    // Conditional B. SWI/UND encodings (cond >= 0xE) remain legacy.
    if instr >> 12 == 0xD && ((instr >> 8) & 0xF) < 0xE {
        let condition = ((instr >> 8) & 0xF) as u8;
        if !condition_passed(regs.cpsr(), condition) {
            return Some(vec![MicroOp::Internal]);
        }
        let offset = ((instr & 0xFF) as i8 as i32) << 1;
        return Some(vec![
            MicroOp::Internal,
            MicroOp::Internal,
            MicroOp::TakenBranch(BranchEffect {
                offset: offset as u32,
                link: false,
            }),
        ]);
    }
    // B (unconditional).
    if instr >> 11 == 0b11100 {
        let offset = ((instr & 0x7FF) as i32) << 1;
        let offset = (offset << 20) >> 20;
        return Some(vec![
            MicroOp::Internal,
            MicroOp::Internal,
            MicroOp::TakenBranch(BranchEffect {
                offset: offset as u32,
                link: false,
            }),
        ]);
    }
    // Long BL: high half sets LR (1 cycle); low half refills like B.
    if instr >> 11 == 0b11110 {
        let offset = ((instr & 0x7FF) as i32) << 12;
        let offset = (offset << 9) >> 9; // sign extend
        return Some(vec![MicroOp::BlHigh(offset as u32)]);
    }
    if instr >> 11 == 0b11111 {
        let offset = ((instr & 0x7FF) as u32) << 1;
        return Some(vec![
            MicroOp::Internal,
            MicroOp::Internal,
            MicroOp::BlLow(offset),
        ]);
    }
    // BX (the whole 0x4700 range is op 0b11): interworking branch.
    if (instr & 0xFF00) == 0x4700 {
        let rs = ((instr >> 3) & 0xF) as usize;
        return Some(vec![
            MicroOp::Internal,
            MicroOp::Internal,
            MicroOp::Bx(regs.r(rs)),
        ]);
    }
    // MOV/CMP/ADD/SUB immediate.
    if instr >> 13 == 0b001 {
        let op = match (instr >> 11) & 0x3 {
            0b00 => AluImmOp::Mov,
            0b01 => AluImmOp::Cmp,
            0b10 => AluImmOp::Add,
            _ => AluImmOp::Sub,
        };
        let rd = ((instr >> 8) & 0x7) as usize;
        let imm = (instr & 0xFF) as u32;
        return Some(vec![MicroOp::CommitAlu(AluEffect {
            op,
            rd,
            rn: rd,
            imm,
            set_flags: true,
            thumb_mov: matches!(op, AluImmOp::Mov),
            carry: None,
        })]);
    }
    // Thumb MUL (op 0xD in 0x4000..=0x43FF): m from the incoming Rd,
    // mirroring the decoder preamble; other ALU ops stay legacy.
    if (instr & 0xFFC0) == 0x4340 {
        let m = multiplier_cycles(regs.r((instr & 0x7) as usize));
        let mut ops = vec![MicroOp::CommitMul(MulEffect {
            instr: u32::from(instr),
            thumb: true,
        })];
        ops.extend(vec![MicroOp::Internal; m as usize]);
        return Some(ops);
    }
    if let Some(ops) = expand_thumb_alu_rest(instr) {
        return Some(ops);
    }
    expand_thumb_push_pop(instr, regs)
        .or_else(|| expand_thumb_multiple(instr, regs))
        .or_else(|| expand_thumb_pcrel(instr, regs))
        .or_else(|| expand_thumb_load_store(instr))
}

/// Thumb LDR (literal) 0x4800..=0x4FFF. Expands to [Read, I, I] (= 3),
/// matching `handle_pc_relative` (bus read32, then fetch-stream-break).
fn expand_thumb_pcrel(instr: u16, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    if !(0x4800..=0x4FFF).contains(&instr) {
        return None;
    }
    let addr = (regs.pc() & !3).wrapping_add(((instr & 0xFF) as u32) << 2);
    Some(vec![
        MicroOp::PcRelRead(PcRelRead {
            addr,
            rd: ((instr >> 8) & 0x7) as usize,
        }),
        MicroOp::Internal,
        MicroOp::Internal,
    ])
}

/// Thumb LDMIA/STMIA, non-empty lists only (empty forms keep the
/// legacy PC-transfer/+0x40 quirk path). Same block slicing as
/// PUSH/POP: per-word continuation at execution, batch open/close and
/// the single fetch-stream break instruction-scoped, base writeback
/// (LDM skips it when the base is loaded) in the end commit.
fn expand_thumb_multiple(instr: u16, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    if instr >> 12 != 0xC {
        return None;
    }
    let load = (instr >> 11) & 1 == 1;
    let rb = ((instr >> 8) & 0x7) as usize;
    let rlist = instr & 0xFF;
    if rlist == 0 {
        return None;
    }
    let base = regs.r(rb);
    let count = rlist.count_ones();
    let slots: Vec<usize> = (0..8).filter(|i| rlist & (1 << i) != 0).collect();
    // STM stored-base quirk: a non-first occurrence of the base stores
    // the final address (legacy `stm_value`).
    let final_addr = base.wrapping_add(count * 4);
    let first = rlist.trailing_zeros() as usize;
    let mut ops = vec![MicroOp::BlockStart(BlockStartEffect {
        is_load: load,
        fetch_width: 2,
    })];
    for (i, reg) in slots.iter().enumerate() {
        let store_value = if load {
            None
        } else {
            Some(if *reg == rb && rb != first {
                final_addr
            } else {
                regs.r(*reg)
            })
        };
        ops.push(MicroOp::BlockWord(BlockWord {
            addr: base.wrapping_add(i as u32 * 4),
            reg: *reg,
            load,
            first: i == 0,
            pc_load: false,
            user_bank: false,
            restore_cpsr: false,
            store_value,
        }));
    }
    // LDM writes the base back only when it is not itself loaded.
    let writeback = if load && (rlist >> rb) & 1 != 0 {
        None
    } else {
        Some((rb, final_addr))
    };
    ops.push(MicroOp::BlockEnd(BlockEndEffect {
        sp: None,
        writeback,
        ldm_conflict: false,
    }));
    // Pad the legacy handler base (LDM 2+count, STM 1+count).
    ops.extend(vec![MicroOp::Internal; if load { 2 } else { 1 }]);
    Some(ops)
}

/// Thumb ALU remainder: move-shifted (0x0000..=0x17FF, 1 cycle),
/// ADD/SUB (0x1800..=0x1FFF, 1 cycle), register ALU (0x4000..=0x43FF
/// except MUL: 1 cycle, 2 for register shifts), hi-reg
/// (0x4400..=0x47FF except BX: 1 cycle, 3 for ADD/MOV to PC),
/// ADD SP/PC (0xA000..=0xAFFF) and SP offset (0xB000..=0xB0FF).
/// Expansion is [CommitThumb] padded to the legacy base; the commit
/// delegates, so only the cycle split is new.
fn expand_thumb_alu_rest(instr: u16) -> Option<Vec<MicroOp>> {
    let trailing: usize = if instr <= 0x1FFF || (0xA000..=0xB0FF).contains(&instr) {
        0
    } else if (0x4000..=0x43FF).contains(&instr) && ((instr >> 6) & 0xF) != 0xD {
        let op = ((instr >> 6) & 0xF) as u8;
        if matches!(op, 0x2..=0x4 | 0x7) { 1 } else { 0 }
    } else if (0x4400..=0x47FF).contains(&instr) && (instr & 0xFF00) != 0x4700 {
        let op = (instr >> 8) & 0b11;
        let rd = (instr & 0x7) + if (instr >> 7) & 1 != 0 { 8 } else { 0 };
        if rd == 15 && (op == 0b00 || op == 0b10) {
            2
        } else {
            0
        }
    } else {
        return None;
    };
    let mut ops = vec![MicroOp::CommitThumb(instr)];
    ops.extend(vec![MicroOp::Internal; trailing]);
    Some(ops)
}

/// Thumb PUSH/POP, non-empty lists only (empty forms keep the legacy
/// quirk path: PC-store / PC-load with +0x40 SP arithmetic). Mirrors
/// the decoder ranges exactly (subset of the legacy route, no extra
/// validation: the handler itself does not validate either).
fn expand_thumb_push_pop(instr: u16, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    let push = match instr {
        0xB400..=0xB5FF => true,
        0xBC00..=0xBDFF => false,
        _ => return None,
    };
    let sp = regs.sp();
    let extra = instr & (1 << 8) != 0; // LR for PUSH, PC for POP
    let list = instr & 0xFF;
    let count = list.count_ones() + u32::from(extra);
    if count == 0 {
        return None;
    }
    let mut slots: Vec<(usize, bool)> = (0..8)
        .filter(|r| list & (1 << r) != 0)
        .map(|r| (r as usize, false))
        .collect();
    if extra {
        slots.push((if push { 14 } else { 15 }, !push));
    }
    let mut ops = vec![MicroOp::BlockStart(BlockStartEffect {
        is_load: !push,
        fetch_width: 2,
    })];
    for (i, (reg, pc_load)) in slots.iter().enumerate() {
        let addr = if push {
            sp.wrapping_sub(count * 4).wrapping_add(i as u32 * 4)
        } else {
            sp.wrapping_add(i as u32 * 4)
        };
        ops.push(MicroOp::BlockWord(BlockWord {
            addr,
            reg: *reg,
            load: !push,
            first: i == 0,
            pc_load: *pc_load,
            user_bank: false,
            restore_cpsr: false,
            store_value: None,
        }));
    }
    ops.push(MicroOp::BlockEnd(BlockEndEffect {
        sp: Some(if push {
            sp.wrapping_sub(count * 4)
        } else {
            sp.wrapping_add(count * 4)
        }),
        writeback: None,
        ldm_conflict: false,
    }));
    // Pad the legacy handler base: PUSH 1+count, POP 2+count,
    // POP+PC 4+count (words already carry +1 each).
    let trailing = if push {
        1
    } else if extra {
        4
    } else {
        2
    };
    ops.extend(vec![MicroOp::Internal; trailing as usize]);
    Some(ops)
}

/// Thumb word LDR/STR (immediate offset and SP-relative) and
/// LDRH/STRH immediate. Same [Read, I, I] / [Write, I] shape as ARM.
pub fn expand_thumb_load_store(instr: u16) -> Option<Vec<MicroOp>> {
    // Register-offset word/byte and signed/halfword transfers (0101).
    if instr >> 12 == 0b0101 {
        let op = (instr >> 10) & 0x3;
        let ro = ((instr >> 6) & 0x7) as usize;
        let rb = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;
        let signed_group = instr & 0x0200 != 0;
        let (load, width, signed_load) = if signed_group {
            match op {
                0b00 => (false, 2, false), // STRH
                0b01 => (true, 1, true),   // LDRSB
                0b10 => (true, 2, false),  // LDRH
                _ => (true, 2, true),      // LDRSH
            }
        } else {
            (
                instr & (1 << 11) != 0,
                if instr & (1 << 10) != 0 { 1 } else { 4 },
                false,
            )
        };
        let acc = MemAccess {
            width,
            rd,
            rn: rb,
            offset: 0,
            offset_register: Some(ro),
            subtract: false,
            is_sp: false,
            signed_load,
            post_indexed: false,
            writeback: false,
            store_value: None,
            // Only the Thumb LDRSH carries the odd-address bus quirk.
            halfword_odd_quirk: load && width == 2 && signed_load,
        };
        return Some(if load {
            vec![MicroOp::MemRead(acc), MicroOp::Internal, MicroOp::Internal]
        } else {
            vec![MicroOp::MemWrite(acc), MicroOp::Internal]
        });
    }
    // SP-relative (1001): addr = SP + imm8<<2.
    if instr >> 12 == 0b1001 {
        let l = (instr >> 11) & 1 == 1;
        let rd = ((instr >> 8) & 0x7) as usize;
        let acc = MemAccess {
            width: 4,
            rd,
            rn: 13,
            offset: ((instr & 0xFF) as u32) << 2,
            offset_register: None,
            subtract: false,
            is_sp: true,
            signed_load: false,
            post_indexed: false,
            writeback: false,
            store_value: None,
            halfword_odd_quirk: false,
        };
        // Totals match legacy handler returns (load 3, store 2).
        return Some(if l {
            vec![MicroOp::MemRead(acc), MicroOp::Internal, MicroOp::Internal]
        } else {
            vec![MicroOp::MemWrite(acc), MicroOp::Internal]
        });
    }
    // Immediate-offset word (011, B == 0).
    if instr >> 13 == 0b011 && (instr >> 12) & 1 == 0 {
        let l = (instr >> 11) & 1 == 1;
        let rb = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;
        let acc = MemAccess {
            width: 4,
            rd,
            rn: rb,
            offset: (((instr >> 6) & 0x1F) as u32) << 2,
            offset_register: None,
            subtract: false,
            is_sp: false,
            signed_load: false,
            post_indexed: false,
            writeback: false,
            store_value: None,
            halfword_odd_quirk: false,
        };
        // Totals match legacy handler returns (load 3, store 2).
        return Some(if l {
            vec![MicroOp::MemRead(acc), MicroOp::Internal, MicroOp::Internal]
        } else {
            vec![MicroOp::MemWrite(acc), MicroOp::Internal]
        });
    }
    // Halfword immediate (1000).
    if instr >> 12 == 0b1000 {
        let l = (instr >> 11) & 1 == 1;
        let rb = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;
        let acc = MemAccess {
            width: 2,
            rd,
            rn: rb,
            offset: (((instr >> 6) & 0x1F) as u32) << 1,
            offset_register: None,
            subtract: false,
            is_sp: false,
            signed_load: false,
            post_indexed: false,
            writeback: false,
            store_value: None,
            halfword_odd_quirk: false,
        };
        // Totals match legacy handler returns (load 3, store 2).
        return Some(if l {
            vec![MicroOp::MemRead(acc), MicroOp::Internal, MicroOp::Internal]
        } else {
            vec![MicroOp::MemWrite(acc), MicroOp::Internal]
        });
    }
    None
}

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

/// Apply one data access with the legacy handler's exact bus-call order
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
                // byte read (legacy `halfword_transfer`).
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
    bus.charge_fetch_stream_break();
}

/// Apply one data store with the legacy handler's exact bus-call order
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
    bus.charge_fetch_stream_break();
}

fn apply_alu(regs: &mut CpuRegisters, fx: AluEffect) {
    // Shifter carry-out: snapshot for S with a rotated immediate, else
    // the live CPSR carry. Arithmetic carry-INS always read the live
    // CPSR carry (legacy `execute` takes both separately; conflating
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
            // Overflow via the exact signed total (legacy formula: a
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
            // N: Thumb MOV-imm forces 0 (legacy quirk); otherwise
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

/// Interpret one pipelined step using expansion. Mirrors
/// `GbaCpu::step_arm/step_thumb` (fetch/rotate/flush/refill) exactly;
/// only the execute phase goes through micro-ops. Returns `None` when
/// the executing instruction is not covered (caller keeps legacy path).
/// `pipeline`/`regs` layout matches `GbaCpu` (`pipeline[0]` executes).
#[allow(dead_code)]
/// Single micro-op step; runs one op per call so the driver can tick peripherals between ops.
/// Returns the op's true cost with no floor (may be <= 0 from prefetch erases); driver floors once at retire.
/// `None` = uncovered fill, queue untouched.
pub fn step_op(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    pipeline: &mut [u32; 2],
    queue: &mut VecDeque<MicroOp>,
    is_thumb: bool,
) -> Option<i64> {
    if queue.is_empty() {
        // Speculative pure decode FIRST (fallback history: touching
        // bus/pipeline before coverage is known double-advances the
        // pipeline on legacy fallback).
        let ops = if is_thumb {
            expand_thumb((pipeline[0] & 0xFFFF) as u16, regs)?
        } else {
            expand_arm(pipeline[0], regs)?
        };
        bus.take_access_wait_cycles();
        bus.set_current_pc(regs.pc());
        let pc = regs.pc();
        let fetched = if is_thumb {
            bus.fetch16(pc) as u32
        } else {
            bus.fetch32(pc)
        };
        pipeline[0] = pipeline[1];
        pipeline[1] = fetched;
        regs.clear_pc_written();
        queue.extend(ops);
    }
    let pc = regs.pc();
    let op = queue.pop_front().expect("expansion never yields zero ops");
    let mut cycles: i64 = 0;
    match op {
        MicroOp::Internal => cycles += 1,
        MicroOp::CommitAlu(fx) => {
            apply_alu(regs, fx);
            cycles += 1;
        }
        MicroOp::CommitThumb(instr) => {
            // Single-cycle ALU remainder (plus padded Internals for
            // the multi-cycle forms): delegate to the matching legacy
            // handler, which performs no bus access.
            match instr {
                0x0000..=0x17FF => move_shifted::handle(regs, instr),
                0x1800..=0x1FFF => add_sub::handle(regs, instr),
                0x4000..=0x43FF => thumb_alu_handle(regs, instr),
                0xA000..=0xAFFF => handle_load_address(regs, instr),
                0xB000..=0xB0FF => handle_sp_offset(regs, instr),
                // Gate guarantees hi-reg non-BX here.
                _ => hi_register::handle(regs, bus, instr),
            };
            cycles += 1;
        }
        MicroOp::CommitDpReg(instr) => {
            // Data-processing performs no bus access: delegate to the
            // legacy handler (frozen oracle) and carry +1; trailing
            // Internals pad the legacy base.
            dp_handle(regs, bus, instr);
            cycles += 1;
        }
        MicroOp::CommitMul(m) => {
            if m.thumb {
                // Mirrors the decoder preamble (fetch break + P-ON
                // erase) plus the ALU handler; m comes from Rd.
                let instr = m.instr as u16;
                let ticks = multiplier_cycles(regs.r((instr & 0x7) as usize));
                bus.charge_fetch_stream_break();
                bus.erase_for_multiply(ticks, 2);
                thumb_alu_handle(regs, instr);
            } else {
                mul_handle(regs, bus, m.instr);
            }
            cycles += 1;
        }
        MicroOp::CommitSwp(instr) => {
            // Atomic read+write inside the legacy handler (bus lock);
            // carry +1, trailing Internals pad the base.
            swp_handle(regs, bus, instr);
            cycles += 1;
        }
        MicroOp::CommitPsr(instr) => {
            psr_handle(regs, instr);
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
        MicroOp::MemRead(a) => {
            // Legacy-identical issue: bus access, writeback and break in
            // the issue tick. (A deferred-commit timer re-sample was tried
            // here and FALSIFIED — it breaks 12 hw-test DMA pins that pin
            // issue-time sampling; see the design doc. The queue/drain
            // machinery stays as the verified-neutral execution model.)
            apply_read(regs, bus, a);
            // Thumb single word-load retire (mirrors the legacy handler
            // hook; the legacy path never runs for covered classes).
            // regs.pc() is the fetch PC here exactly as in step_thumb,
            // so execute-PC adjacency validates the same way.
            if is_thumb && a.width == 4 && !a.signed_load {
                let (addr, _) = resolve_addr(regs, a);
                bus.note_thumb_single_load(regs.pc(), addr);
            }
            cycles += 1;
        }
        MicroOp::MemWrite(a) => {
            apply_write(regs, bus, a);
            cycles += 1;
        }
        MicroOp::PcRelRead(r) => {
            // Legacy-identical order: bus access, then fetch-stream-break.
            regs.set_r(r.rd, bus.read32(r.addr));
            bus.charge_fetch_stream_break();
            // Thumb literal retire (loads the marker chain like a load).
            if is_thumb {
                bus.note_thumb_single_load(regs.pc(), r.addr);
            }
            cycles += 1;
        }
        MicroOp::BlockStart(e) => {
            bus.begin_block_batch(e.is_load, e.fetch_width);
        }
        MicroOp::BlockWord(w) => {
            // Legacy-identical per-word order: continuation query, then
            // the access. A DMA burst between words resets the address
            // stream (next word N), same as post-DMA CPU accesses.
            let continuation = !w.first && bus.data_continuation_sequential(w.addr);
            bus.set_data_sequential(continuation);
            if w.load {
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
            cycles += 1;
        }
        MicroOp::BlockEnd(e) => {
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
            bus.charge_fetch_stream_break();
        }
    }
    if queue.is_empty() {
        if regs.take_pc_written() {
            *pipeline = [0; 2];
            bus.set_current_pc(regs.pc());
            bus.invalidate_prefetch_for_branch();
            fill_pipeline(regs, bus, pipeline);
            // Legacy returns `cycles` here (plus an IRQ epilogue only on
            // the trampoline path, out of scope).
        } else {
            regs.set_pc(pc.wrapping_add(if is_thumb { 2 } else { 4 }));
        }
    }
    // No floor here: the driver floors once per instruction at retire,
    // exactly like the legacy step (per-op flooring would inflate
    // prefetch-erased instructions).
    Some(cycles + bus.take_access_wait_cycles())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::GbaCpu;

    /// Run `steps` instructions under both engines from identical state.
    /// Returns (legacy_total, micro_total, regs_equal, followup_equal).
    /// `code_base` locates the corpus (IWRAM for writable code, ROM with
    /// `cart` for GamePak-code paths); `mem_init`/`reg_init` preset memory
    /// (addr, width, value) and registers before the pipeline fill.
    #[allow(clippy::too_many_arguments)]
    fn differential(
        code: &[u32],
        thumb: bool,
        waitcnt: u16,
        steps: usize,
        followup: u32,
        code_base: u32,
        cart: Option<Vec<u8>>,
        mem_init: &[(u32, u8, u32)],
        reg_init: &[(usize, u32)],
    ) -> (u32, u32, bool, bool) {
        fn setup(
            code: &[u32],
            thumb: bool,
            waitcnt: u16,
            code_base: u32,
            cart: Option<Vec<u8>>,
            mem_init: &[(u32, u8, u32)],
            reg_init: &[(usize, u32)],
        ) -> (GbaCpu, GbaMemoryBus, [u32; 2]) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write16(0x04000204, waitcnt);
            let stride = if thumb { 2 } else { 4 };
            if let Some(mut rom) = cart {
                for (i, w) in code.iter().enumerate() {
                    let o = 0x100 + i * stride as usize;
                    if thumb {
                        rom[o] = (w & 0xFF) as u8;
                        rom[o + 1] = (w >> 8) as u8;
                    } else {
                        rom[o..o + 4].copy_from_slice(&w.to_le_bytes());
                    }
                }
                bus.set_cartridge(crate::cartridge::Cartridge::new(rom).unwrap());
            } else {
                for (i, w) in code.iter().enumerate() {
                    let addr = code_base + (i as u32) * stride as u32;
                    if thumb {
                        bus.write16(addr, (w & 0xFFFF) as u16);
                    } else {
                        bus.write32(addr, *w);
                    }
                }
            }
            for (addr, width, val) in mem_init {
                match width {
                    4 => bus.write32(*addr, *val),
                    2 => bus.write16(*addr, (*val & 0xFFFF) as u16),
                    _ => bus.write8(*addr, (*val & 0xFF) as u8),
                }
            }
            for (r, v) in reg_init {
                cpu.regs.set_r(*r, *v);
            }
            // Stack for the follow-up load.
            bus.write16(0x0300_7F00, 0x1234);
            cpu.regs.set_r(13, 0x0300_7F00);
            if thumb {
                cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
            }
            cpu.regs.set_pc(code_base);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            // Shadow pipeline for the interpreter (same fetches).
            let mut shadow = [0u32; 2];
            // Reproduce the fill through the same bus calls: reset and
            // refill identically by re-driving fill on a twin bus is
            // overkill; instead mirror the two fetch values.
            shadow[0] = cpu.pipeline[0];
            shadow[1] = cpu.pipeline[1];
            (cpu, bus, shadow)
        }
        let (mut a_cpu, mut a_bus, _) = setup(
            code,
            thumb,
            waitcnt,
            code_base,
            cart.clone(),
            mem_init,
            reg_init,
        );
        let (mut b_cpu, mut b_bus, mut b_pipe) =
            setup(code, thumb, waitcnt, code_base, cart, mem_init, reg_init);
        // Twin-bus check: both setups must agree before stepping.
        assert_eq!(a_cpu.pipeline, b_pipe);
        let (mut ta, mut tb) = (0u32, 0u32);
        let mut b_queue = std::collections::VecDeque::new();
        for _ in 0..steps {
            // Legacy oracle: step_legacy bypasses the micro-op wiring so
            // the harness stays a true differential even once covered
            // classes route through step_op in production.
            ta += a_cpu.step_legacy(&mut a_bus);
            // Micro-op engine on the twin: drain one full instruction
            // (the queue may span several step_op calls), flooring once
            // at retire exactly like the legacy step.
            let mut acc = 0i64;
            loop {
                acc += step_op(
                    &mut b_cpu.regs,
                    &mut b_bus,
                    &mut b_pipe,
                    &mut b_queue,
                    thumb,
                )
                .expect("corpus must be covered");
                if b_queue.is_empty() {
                    break;
                }
            }
            tb += acc.max(1) as u32;
        }
        let regs_equal = (0..16).all(|r| a_cpu.regs.r(r) == b_cpu.regs.r(r))
            && a_cpu.regs.cpsr() == b_cpu.regs.cpsr();
        // Follow-up load through the legacy engine on both buses: detects
        // fetch-stream/erase-state divergence.
        b_cpu.pipeline = b_pipe;
        a_cpu.regs.set_pc(0x0300_0000);
        b_cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut a_cpu.regs, &mut a_bus, &mut a_cpu.pipeline);
        fill_pipeline(&mut b_cpu.regs, &mut b_bus, &mut b_cpu.pipeline);
        a_bus.take_access_wait_cycles();
        b_bus.take_access_wait_cycles();
        // Point both at the follow-up instruction.
        let fa = run_one_legacy(&mut a_cpu, &mut a_bus, followup, thumb);
        let fb = run_one_legacy(&mut b_cpu, &mut b_bus, followup, thumb);
        (ta, tb, regs_equal, fa == fb)
    }

    /// Execute one arbitrary instruction via the legacy engine (bypasses
    /// the micro-op wiring like the differential oracle).
    fn run_one_legacy(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, word: u32, thumb: bool) -> u32 {
        let base = 0x0300_0100u32;
        if thumb {
            bus.write16(base, (word & 0xFFFF) as u16);
            bus.write16(base + 2, (word >> 16) as u16);
            cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
        } else {
            bus.write32(base, word);
        }
        cpu.regs.set_pc(base);
        fill_pipeline(&mut cpu.regs, bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        cpu.step_legacy(bus)
    }

    // Corpus: ALU-imm + B, ARM and Thumb. Encodings hand-checked against
    // the existing handler unit tests.
    const ARM_CORPUS: [u32; 6] = [
        0xE3A0_0001, // mov r0, #1
        0xE281_1002, // add r1, r1, #2
        0xE252_2003, // subs r2, r2, #3
        0xE354_0004, // cmp r4, #4
        0xEA00_0001, // b +12 (skips one)
        0xE3A0_3005, // mov r3, #5
    ];
    const THUMB_CORPUS: [u32; 6] = [
        0x2001, // mov r0, #1
        0x3102, // add r1, #2
        0x3A03, // sub r2, #3
        0x2C04, // cmp r4, #4
        0xE001, // b +4 (skips one)
        0x2305, // mov r3, #5
    ];

    #[test]
    fn micro_op_matches_legacy_arm() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4014] {
            let (ta, tb, regs, follow) = differential(
                &ARM_CORPUS,
                false,
                waitcnt,
                5,
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[],
                &[],
            );
            assert_eq!((ta, tb), (ta, ta), "waitcnt={waitcnt:#06x}");
            assert!(regs, "waitcnt={waitcnt:#06x}");
            assert!(follow, "waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn micro_op_matches_legacy_thumb() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4014] {
            let (ta, tb, regs, follow) = differential(
                &THUMB_CORPUS,
                true,
                waitcnt,
                5,
                0x886A,
                0x0300_0000,
                None,
                &[],
                &[],
            );
            assert_eq!((ta, tb), (ta, ta), "waitcnt={waitcnt:#06x}");
            assert!(regs, "waitcnt={waitcnt:#06x}");
            assert!(follow, "waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn conditional_branch_loops_match_legacy() {
        let arm = [
            0xE3A0_0000, // mov r0, #0
            0xE280_0001, // add r0, r0, #1
            0xE350_0003, // cmp r0, #3
            0x1AFF_FFFC, // bne back to add
            0xE3A0_1007, // mov r1, #7
        ];
        let thumb = [
            0x2000, // mov r0, #0
            0x3001, // add r0, #1
            0x2803, // cmp r0, #3
            0xD1FC, // bne back to add
            0x2107, // mov r1, #7
        ];
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs, follow) = differential(
                &arm,
                false,
                waitcnt,
                11,
                0xE3A0_2009,
                0x0300_0000,
                None,
                &[],
                &[],
            );
            assert_eq!((ta, tb), (ta, ta), "ARM waitcnt={waitcnt:#06x}");
            assert!(regs && follow, "ARM waitcnt={waitcnt:#06x}");

            let (ta, tb, regs, follow) = differential(
                &thumb,
                true,
                waitcnt,
                11,
                0x2209,
                0x0300_0000,
                None,
                &[],
                &[],
            );
            assert_eq!((ta, tb), (ta, ta), "Thumb waitcnt={waitcnt:#06x}");
            assert!(regs && follow, "Thumb waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn conditional_alu_and_memory_match_legacy() {
        let code = [
            0xE3A0_0000, // mov r0, #0
            0xE350_0000, // cmp r0, #0 (Z=1)
            0x0281_1001, // addeq r1, r1, #1
            0x1282_2001, // addne r2, r2, #1 (skipped)
            0x0583_1000, // streq r1, [r3]
            0x1593_4000, // ldrne r4, [r3] (skipped)
            0xE593_5000, // ldr r5, [r3]
        ];
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs, follow) = differential(
                &code,
                false,
                waitcnt,
                code.len(),
                0xE3A0_6009,
                0x0300_0000,
                None,
                &[],
                &[(1, 4), (2, 8), (3, 0x0200_0100)],
            );
            assert_eq!((ta, tb), (ta, ta), "waitcnt={waitcnt:#06x}");
            assert!(regs && follow, "waitcnt={waitcnt:#06x}");
        }
    }

    // Slice 2 corpus: single load/store (word + halfword, ARM + Thumb).
    // Pointers preset in Rust; r0 carries the store value.
    const ARM_LS_CORPUS: [u32; 8] = [
        0xE3A0_1402, // mov r1, #0x02000000 (EWRAM ptr; rot, S=0)
        0xE3A0_1403, // mov r2, #0x03000000 (IWRAM ptr; rot, S=0)
        0xE581_0004, // str r0, [r1, #4]
        0xE591_3004, // ldr r3, [r1, #4]
        0xE1C2_00B8, // strh r0, [r2, #8]
        0xE1D2_40B8, // ldrh r4, [r2, #8]
        0xE591_5008, // ldr r5, [r1, #8]
        0xE3A0_0007, // mov r7, #7
    ];
    const ARM_INDEXED_LS_CORPUS: [u32; 8] = [
        0xE4C1_0001, // strb r0, [r1], #1
        0xE551_2001, // ldrb r2, [r1, #-1]
        0xE5E1_0002, // strb r0, [r1, #2]!
        0xE411_3004, // ldr r3, [r1], #-4
        0xE0C1_00B2, // strh r0, [r1], #2
        0xE171_40B2, // ldrh r4, [r1, #-2]!
        0xE591_5000, // ldr r5, [r1]
        0xE3A0_6007, // mov r6, #7
    ];
    const THUMB_LS_CORPUS: [u32; 7] = [
        0x6088, // str r0, [r1, #8]
        0x688B, // ldr r3, [r1, #8]
        0x8090, // strh r0, [r2, #4]
        0x8894, // ldrh r4, [r2, #4]
        0x9010, // str r0, [sp, #0x40]
        0x9D10, // ldr r5, [sp, #0x40]
        0x2707, // mov r7, #7
    ];
    const THUMB_REG_LS_CORPUS: [u32; 8] = [
        0x5088, // str r0, [r1, r2]
        0x588B, // ldr r3, [r1, r2]
        0x5488, // strb r0, [r1, r2]
        0x5C8C, // ldrb r4, [r1, r2]
        0x5288, // strh r0, [r1, r2]
        0x5A8D, // ldrh r5, [r1, r2]
        0x568E, // ldrsb r6, [r1, r2]
        0x5E8F, // ldrsh r7, [r1, r2]
    ];
    // ROM-code corpus: same shapes from GamePak (prefetch/erase paths)
    // plus a ROM-data read. Code at 0x08000100, data word at 0x08000200.
    const ARM_ROM_CORPUS: [u32; 9] = [
        0xE3A0_1402, // mov r1, #0x02000000
        0xE3A0_1403, // mov r2, #0x03000000
        0xE3A0_1408, // mov r4, #0x08000000 (ROM ptr; rot, S=0)
        0xE581_0004, // str r0, [r1, #4]
        0xE591_3004, // ldr r3, [r1, #4]
        0xE1C2_00B8, // strh r0, [r2, #8]
        0xE1D2_50B8, // ldrh r5, [r2, #8]
        0xE594_6000, // ldr r6, [r4, #0] (ROM data)
        0xE3A0_0007, // mov r7, #7
    ];

    fn rom_cart() -> Vec<u8> {
        const SUITE: &[u8] = include_bytes!("../../../../roms/gba/mgba-suite/suite.gba");
        let mut rom = vec![0u8; 0x10000];
        rom[..0xC0].copy_from_slice(&SUITE[..0xC0]);
        rom[0x200..0x204].copy_from_slice(&0xCAFE_BABEu32.to_le_bytes());
        rom
    }

    #[test]
    fn micro_op_load_store_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs, follow) = differential(
                &ARM_LS_CORPUS,
                false,
                waitcnt,
                8,
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[(0x0200_0008, 4, 0xAABB_CCDD)],
                &[(0, 0x1234_5678)],
            );
            assert_eq!((ta, tb), (ta, ta), "arm waitcnt={waitcnt:#06x}");
            assert!(regs, "arm waitcnt={waitcnt:#06x}");
            assert!(follow, "arm waitcnt={waitcnt:#06x}");
            let (ta, tb, regs, follow) = differential(
                &ARM_INDEXED_LS_CORPUS,
                false,
                waitcnt,
                ARM_INDEXED_LS_CORPUS.len(),
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[],
                &[(0, 0x1234_80FF), (1, 0x0300_0200)],
            );
            assert_eq!((ta, tb), (ta, ta), "arm-indexed waitcnt={waitcnt:#06x}");
            assert!(regs, "arm-indexed waitcnt={waitcnt:#06x}");
            assert!(follow, "arm-indexed waitcnt={waitcnt:#06x}");
            let (ta, tb, regs, follow) = differential(
                &THUMB_LS_CORPUS,
                true,
                waitcnt,
                7,
                0x886A,
                0x0300_0000,
                None,
                &[(0x0200_0008, 4, 0xAABB_CCDD)],
                &[(0, 0x1234_5678), (1, 0x0200_0000), (2, 0x0300_0300)],
            );
            assert_eq!((ta, tb), (ta, ta), "thumb waitcnt={waitcnt:#06x}");
            assert!(regs, "thumb waitcnt={waitcnt:#06x}");
            assert!(follow, "thumb waitcnt={waitcnt:#06x}");

            let (ta, tb, regs, follow) = differential(
                &THUMB_REG_LS_CORPUS,
                true,
                waitcnt,
                THUMB_REG_LS_CORPUS.len(),
                0x886A,
                0x0300_0000,
                None,
                &[],
                &[(0, 0x0000_80FF), (1, 0x0300_0100), (2, 4)],
            );
            assert_eq!((ta, tb), (ta, ta), "thumb-reg waitcnt={waitcnt:#06x}");
            assert!(regs, "thumb-reg waitcnt={waitcnt:#06x}");
            assert!(follow, "thumb-reg waitcnt={waitcnt:#06x}");
        }
    }

    // Slice 3 corpus: Thumb PUSH/POP (non-empty; empty forms stay
    // legacy). The harness presets SP=0x03007F00; pushes spill below
    // and pops reload them (LIFO-balanced across the corpus).
    const THUMB_PP_CORPUS: [u32; 5] = [
        0xB40F, // push {r0-r3}
        0xBCF0, // pop {r4-r7}
        0xB550, // push {r4,r6,lr}
        0xBC0B, // pop {r0,r1,r3}
        0x2707, // mov r7, #7
    ];

    #[test]
    fn micro_op_push_pop_matches_legacy() {
        let regs = [
            (0, 0x1111_1111),
            (1, 0x2222_2222),
            (2, 0x3333_3333),
            (3, 0x4444_4444),
            (4, 0x5555_5555),
            (6, 0x6666_6666),
            (14, 0xDEAD_BEEF),
        ];
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_PP_CORPUS,
                true,
                waitcnt,
                THUMB_PP_CORPUS.len(),
                0x886A,
                0x0300_0000,
                None,
                &[],
                &regs,
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        // ROM-code variant: words still hit IWRAM, but the block batch
        // erase path is the ROM-code one (whole-word total accounting).
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_PP_CORPUS,
                true,
                waitcnt,
                THUMB_PP_CORPUS.len(),
                0x886A,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &regs,
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn empty_push_pop_stays_legacy() {
        fn regs_with_sp(sp: u32) -> CpuRegisters {
            let mut regs = CpuRegisters::post_bios();
            regs.set_sp(sp);
            regs
        }
        let regs = regs_with_sp(0x0300_7F00);
        assert!(expand_thumb(0xB400, &regs).is_none());
        assert!(expand_thumb(0xBC00, &regs).is_none());
        let ops = expand_thumb(0xB40F, &regs).expect("non-empty pushes expand");
        // BlockStart + 4 words + BlockEnd + 1 trailing = 7.
        assert_eq!(ops.len(), 7);
    }

    /// PUSH/POP memory effects under both engines from identical state:
    /// stack words as well as registers and totals must agree.
    #[test]
    fn push_pop_memory_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            let code = THUMB_PP_CORPUS;
            for (i, w) in code.iter().enumerate() {
                bus.write16(0x0300_0000 + (i as u32) * 2, (*w & 0xFFFF) as u16);
            }
            for (r, v) in [
                (0, 0x1111_1111),
                (1, 0x2222_2222),
                (2, 0x3333_3333),
                (3, 0x4444_4444),
            ] {
                cpu.regs.set_r(r, v);
            }
            cpu.regs.set_r(4, 0x5555_5555);
            cpu.regs.set_r(6, 0x6666_6666);
            cpu.regs.set_lr(0xDEAD_BEEF);
            cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
            let mut total = 0u32;
            let mut queue = std::collections::VecDeque::new();
            for _ in 0..steps {
                let mut acc = 0i64;
                loop {
                    acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, true)
                        .expect("corpus must be covered");
                    if queue.is_empty() {
                        break;
                    }
                }
                total += acc.max(1) as u32;
            }
            total
        }
        let (mut a_cpu, mut a_bus) = setup();
        let (mut b_cpu, mut b_bus) = setup();
        let mut ta = 0u32;
        for _ in 0..THUMB_PP_CORPUS.len() {
            ta += a_cpu.step_legacy(&mut a_bus);
        }
        let tb = drain_micro(&mut b_cpu, &mut b_bus, THUMB_PP_CORPUS.len());
        assert_eq!((ta, tb), (ta, ta));
        for r in 0..16 {
            assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
        }
        assert_eq!(a_cpu.regs.cpsr(), b_cpu.regs.cpsr());
        for addr in (0x0300_7EE0..0x0300_7F00).step_by(4) {
            assert_eq!(a_bus.read32(addr), b_bus.read32(addr), "stack {addr:#010X}");
        }
    }

    /// POP {..,PC} through micro-ops: PC lands masked, SP advances past
    /// both words, and the retire flushes the pipeline at the target.
    #[test]
    fn pop_pc_matches_legacy_and_flushes() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write16(0x0300_0000, 0xBD02); // pop {r1, pc}
            bus.write32(0x0300_7EF8, 0xCAFE_BABE);
            bus.write32(0x0300_7EFC, 0x0300_0041); // bit0 set: Thumb stays
            bus.write16(0x0300_0040, 0x2707); // mov r7, #7 (target)
            cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
            cpu.regs.set_pc(0x0300_0000);
            cpu.regs.set_sp(0x0300_7EF8);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                true,
            )
            .expect("pop-pc must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(a_cpu.regs.r(1), 0xCAFE_BABE);
        assert_eq!(b_cpu.regs.r(1), 0xCAFE_BABE);
        assert_eq!(b_cpu.regs.pc(), 0x0300_0044); // target + Thumb pipeline lead
        assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
        assert_eq!(b_cpu.regs.sp(), 0x0300_7F00);
        assert_eq!(b_cpu.regs.sp(), a_cpu.regs.sp());
    }

    #[test]
    fn push_pop_tick_parity() {
        let regs = [
            (0, 0x1111_1111),
            (1, 0x2222_2222),
            (2, 0x3333_3333),
            (3, 0x4444_4444),
            (4, 0x5555_5555),
            (6, 0x6666_6666),
            (14, 0xDEAD_BEEF),
        ];
        let (at, bt, av, bv) = tick_parity(
            &THUMB_PP_CORPUS,
            true,
            0x0000,
            0x0300_0000,
            &regs,
            THUMB_PP_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 4 corpus: Thumb LDMIA/STMIA (non-empty; empty forms stay
    // legacy). Bases preset in Rust; r1/r2 start at A, r3 at A+16.
    // The corpus exercises the STM stored-base quirk (r1 in its own
    // list) and the LDM base-in-list no-writeback rule (r3).
    const THUMB_MULT_CORPUS: [u32; 5] = [
        0xC10F, // stmia r1!, {r0-r3}
        0xCBF0, // ldmia r2!, {r4-r7}
        0xC205, // stmia r2!, {r2,r0} (stored-base quirk)
        0xCB18, // ldmia r3, {r3,r4} (base in list: no writeback)
        0x2707, // mov r7, #7
    ];
    const THUMB_MULT_REGS: [(usize, u32); 7] = [
        (0, 0x1111_1111),
        (1, 0x0300_0100),
        (2, 0x0300_0100),
        (3, 0x0300_0110),
        (4, 0x4444_4444),
        (5, 0x5555_5555),
        (6, 0x6666_6666),
    ];

    #[test]
    fn micro_op_ldm_stm_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_MULT_CORPUS,
                true,
                waitcnt,
                THUMB_MULT_CORPUS.len(),
                0x886A,
                0x0300_0000,
                None,
                &[],
                &THUMB_MULT_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        // ROM-code variant: data still hits IWRAM, batch erase is the
        // ROM-code whole-word path.
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_MULT_CORPUS,
                true,
                waitcnt,
                THUMB_MULT_CORPUS.len(),
                0x886A,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &THUMB_MULT_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn empty_ldm_stm_stays_legacy() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(1, 0x0300_0100);
        assert!(expand_thumb(0xC000, &regs).is_none());
        assert!(expand_thumb(0xC800, &regs).is_none());
        let ops = expand_thumb(0xC10F, &regs).expect("non-empty stmia expands");
        // BlockStart + 4 words + BlockEnd + 1 trailing = 7.
        assert_eq!(ops.len(), 7);
    }

    /// LDM/STM memory effects under both engines: block words as well
    /// as registers and totals must agree.
    #[test]
    fn ldm_stm_memory_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            for (i, w) in THUMB_MULT_CORPUS.iter().enumerate() {
                bus.write16(0x0300_0000 + (i as u32) * 2, (*w & 0xFFFF) as u16);
            }
            for (r, v) in THUMB_MULT_REGS {
                cpu.regs.set_r(r, v);
            }
            cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
            let mut total = 0u32;
            let mut queue = std::collections::VecDeque::new();
            for _ in 0..steps {
                let mut acc = 0i64;
                loop {
                    acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, true)
                        .expect("corpus must be covered");
                    if queue.is_empty() {
                        break;
                    }
                }
                total += acc.max(1) as u32;
            }
            total
        }
        let (mut a_cpu, mut a_bus) = setup();
        let (mut b_cpu, mut b_bus) = setup();
        let mut ta = 0u32;
        for _ in 0..THUMB_MULT_CORPUS.len() {
            ta += a_cpu.step_legacy(&mut a_bus);
        }
        let tb = drain_micro(&mut b_cpu, &mut b_bus, THUMB_MULT_CORPUS.len());
        assert_eq!((ta, tb), (ta, ta));
        for r in 0..16 {
            assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
        }
        assert_eq!(a_cpu.regs.cpsr(), b_cpu.regs.cpsr());
        for addr in (0x0300_0100..0x0300_0130).step_by(4) {
            assert_eq!(a_bus.read32(addr), b_bus.read32(addr), "block {addr:#010X}");
        }
    }

    #[test]
    fn ldm_stm_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &THUMB_MULT_CORPUS,
            true,
            0x0000,
            0x0300_0000,
            &THUMB_MULT_REGS,
            THUMB_MULT_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 5a corpus: ARM LDM/STM (non-empty, S=0; empty forms stay
    // legacy; S-bit forms are covered by the slice-14 corpus below).
    // This corpus exercises IA/DB modes, the STM stored-base quirk
    // (r0 in its own list) and the LDM base-in-list no-writeback rule.
    const ARM_BLOCK_CORPUS: [u32; 6] = [
        0xE8A0_0006, // stmia r0!, {r1,r2}
        0xE8B5_0018, // ldmia r5!, {r3,r4}
        0xE925_0006, // stmdb r5!, {r1,r2}
        0xE8A0_0003, // stmia r0!, {r1,r0} (stored-base quirk)
        0xE8B0_0009, // ldmia r0, {r0,r3} (base in list: no writeback)
        0xE3A0_7007, // mov r7, #7
    ];
    const ARM_BLOCK_REGS: [(usize, u32); 5] = [
        (0, 0x0200_0000),
        (1, 0x1111_1111),
        (2, 0x2222_2222),
        (5, 0x0200_0000),
        (6, 0x0300_0400),
    ];

    #[test]
    fn micro_op_arm_block_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_BLOCK_CORPUS,
                false,
                waitcnt,
                ARM_BLOCK_CORPUS.len(),
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[],
                &ARM_BLOCK_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        // ROM-code variant: data still hits EWRAM, batch erase is the
        // ROM-code whole-word path (fetch_width 4).
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_BLOCK_CORPUS,
                false,
                waitcnt,
                ARM_BLOCK_CORPUS.len(),
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &ARM_BLOCK_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn arm_block_gates_stay_legacy() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(0, 0x0200_0000);
        // S bit now expands (user-bank forms, asserted below); only the
        // empty list stays legacy.
        let ops = expand_arm(0xE8B5_0018 | (1 << 22), &regs).expect("ldmia^ expands");
        // BlockStart + 2 words + BlockEnd + 2 trailing = 6.
        assert_eq!(ops.len(), 6);
        // Empty list (plain and S-bit).
        assert!(expand_arm(0xE8A0_0000, &regs).is_none());
        assert!(expand_arm(0xE8A0_0000 | (1 << 22), &regs).is_none());
        let ops = expand_arm(0xE8A0_0006, &regs).expect("plain stmia expands");
        // BlockStart + 2 words + BlockEnd + 1 trailing = 5.
        assert_eq!(ops.len(), 5);
    }

    /// ARM block memory effects under both engines: cell words as well
    /// as registers and totals must agree.
    #[test]
    fn arm_block_memory_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            for (i, w) in ARM_BLOCK_CORPUS.iter().enumerate() {
                bus.write32(0x0300_0000 + (i as u32) * 4, *w);
            }
            for (r, v) in ARM_BLOCK_REGS {
                cpu.regs.set_r(r, v);
            }
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
            let mut total = 0u32;
            let mut queue = std::collections::VecDeque::new();
            for _ in 0..steps {
                let mut acc = 0i64;
                loop {
                    acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                        .expect("corpus must be covered");
                    if queue.is_empty() {
                        break;
                    }
                }
                total += acc.max(1) as u32;
            }
            total
        }
        let (mut a_cpu, mut a_bus) = setup();
        let (mut b_cpu, mut b_bus) = setup();
        let mut ta = 0u32;
        for _ in 0..ARM_BLOCK_CORPUS.len() {
            ta += a_cpu.step_legacy(&mut a_bus);
        }
        let tb = drain_micro(&mut b_cpu, &mut b_bus, ARM_BLOCK_CORPUS.len());
        assert_eq!((ta, tb), (ta, ta));
        for r in 0..16 {
            assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
        }
        assert_eq!(a_cpu.regs.cpsr(), b_cpu.regs.cpsr());
        for addr in (0x0200_0000..0x0200_0020).step_by(4) {
            assert_eq!(a_bus.read32(addr), b_bus.read32(addr), "cell {addr:#010X}");
        }
    }

    /// ARM LDM with PC in the list: PC lands word-aligned, the retire
    /// flushes the pipeline at the target (+ ARM lead).
    #[test]
    fn arm_ldm_pc_matches_legacy_and_flushes() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write32(0x0300_0000, 0xE890_8000); // ldmia r0, {r15} (no writeback)
            bus.write32(0x0200_0000, 0x0200_0100);
            bus.write32(0x0200_0100, 0xE3A0_7007); // mov r7, #7 (target)
            cpu.regs.set_r(0, 0x0200_0000);
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                false,
            )
            .expect("ldm-pc must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(b_cpu.regs.pc(), 0x0200_0108); // target + ARM pipeline lead
        assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
        assert_eq!(b_cpu.regs.r(0), 0x0200_0000);
        assert_eq!(b_cpu.regs.r(0), a_cpu.regs.r(0));
    }

    #[test]
    fn arm_block_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &ARM_BLOCK_CORPUS,
            false,
            0x4014,
            0x0300_0000,
            &ARM_BLOCK_REGS,
            ARM_BLOCK_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 6 corpus: Thumb LDR (literal). Both loads target the
    // pool word at 0x0C (the second sits at pc=0x06, pinning the
    // `pc & !3` alignment); the tail never executes the pool.
    const THUMB_PCREL_CORPUS: [u32; 6] = [
        0x4801, // ldr r0, [pc, #4] -> 0x0C
        0x4901, // ldr r1, [pc, #4] -> 0x0C (pc=0x06, aligned down)
        0x2707, // mov r7, #7
        0x2000, // (padding)
        0xBEEF, // pool lo
        0xDEAD, // pool hi -> 0xDEADBEEF
    ];

    #[test]
    fn micro_op_pcrel_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_PCREL_CORPUS,
                true,
                waitcnt,
                3,
                0x886A,
                0x0300_0000,
                None,
                &[],
                &[],
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        // ROM-code variant: pool word in ROM (prefetch-window path).
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_PCREL_CORPUS,
                true,
                waitcnt,
                3,
                0x886A,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &[],
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    /// LDR-literal value and alignment: an odd-halfword instruction
    /// must align pc down before adding the offset.
    #[test]
    fn pcrel_load_value_and_align() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write16(0x0300_0000, 0x4800); // (padding)
            bus.write16(0x0300_0002, 0x4901); // ldr r1, [pc, #4]
            bus.write32(0x0300_0008, 0xCAFE_BABE);
            cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
            cpu.regs.set_pc(0x0300_0002);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        // pc=0x06 at execute, &!3=0x04, +4 -> 0x08 pool.
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                true,
            )
            .expect("pcrel must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(a_cpu.regs.r(1), 0xCAFE_BABE);
        assert_eq!(b_cpu.regs.r(1), 0xCAFE_BABE);
    }

    #[test]
    fn pcrel_tick_parity() {
        let (at, bt, av, bv) = tick_parity(&THUMB_PCREL_CORPUS, true, 0x0000, 0x0300_0000, &[], 3);
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 7 corpus: ARM data-processing (register form). Immediate
    // shifts (LSL/LSR), a register shift (+1I), flag-only TST, and S
    // set/clear across opcodes.
    const ARM_DPREG_CORPUS: [u32; 7] = [
        0xE1A0_0001, // mov r0, r1
        0xE081_0002, // add r0, r1, r2
        0xE051_0002, // subs r0, r1, r2
        0xE111_0002, // tst r1, r2
        0xE1B0_0213, // mov r0, r3, lsl r2 (register shift)
        0xE1A0_0122, // mov r0, r2, lsr #2
        0xE3A0_3005, // mov r3, #5
    ];
    const ARM_DPREG_REGS: [(usize, u32); 3] = [(1, 0x100), (2, 4), (3, 0xFF)];

    #[test]
    fn micro_op_arm_dpreg_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_DPREG_CORPUS,
                false,
                waitcnt,
                ARM_DPREG_CORPUS.len(),
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[],
                &ARM_DPREG_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_DPREG_CORPUS,
                false,
                waitcnt,
                ARM_DPREG_CORPUS.len(),
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &ARM_DPREG_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn arm_dpreg_gates_stay_legacy() {
        let regs = CpuRegisters::post_bios();
        // The DP-register branch itself must not misclaim multiply,
        // SWP, MRS/MSR, BX or halfword shapes (they have their own
        // branches or stay legacy); assert on the branch directly.
        for instr in [
            0xE002_0091, // mul
            0xE102_0091, // swp
            0xE10F_0000, // mrs
            0xE129_F000, // msr
            0xE12F_FF11, // bx
            0xE112_00F3, // ldrsh-reg
        ] {
            assert!(expand_arm_dp_reg(instr, &regs).is_none(), "{instr:#010X}");
        }
        let ops = expand_arm(0xE081_0002, &regs).expect("add-reg expands");
        // Commit + 0 trailing = 1.
        assert_eq!(ops.len(), 1);
        let ops = expand_arm(0xE1B0_0213, &regs).expect("reg-shift expands");
        // Commit + 1I = 2.
        assert_eq!(ops.len(), 2);
    }

    #[test]
    fn arm_dpreg_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &ARM_DPREG_CORPUS,
            false,
            0x4014,
            0x0300_0000,
            &ARM_DPREG_REGS,
            ARM_DPREG_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 8 corpus: ARM multiply (short MUL/MLA/S, long UMULL).
    const ARM_MUL_CORPUS: [u32; 5] = [
        0xE000_0291, // mul r0, r1, r2
        0xE022_0391, // mla r2, r1, r3, r0
        0xE001_0291, // muls r0, r1, r2
        0xE083_2190, // umull r3, r2, r0, r1
        0xE3A0_7007, // mov r7, #7
    ];
    const ARM_MUL_REGS: [(usize, u32); 4] = [(0, 3), (1, 4), (2, 7), (3, 5)];

    #[test]
    fn micro_op_arm_mul_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_MUL_CORPUS,
                false,
                waitcnt,
                ARM_MUL_CORPUS.len(),
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[],
                &ARM_MUL_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_MUL_CORPUS,
                false,
                waitcnt,
                ARM_MUL_CORPUS.len(),
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &ARM_MUL_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn arm_mul_padding_matches_legacy_base() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(2, 7);
        // MUL m=1: commit + 1I.
        let ops = expand_arm(0xE000_0291, &regs).expect("mul expands");
        assert_eq!(ops.len(), 2);
        // MLA m=1: commit + 2I.
        let ops = expand_arm(0xE022_0391, &regs).expect("mla expands");
        assert_eq!(ops.len(), 3);
        // UMULL ticks=1: commit + 2I.
        let ops = expand_arm(0xE083_2190, &regs).expect("umull expands");
        assert_eq!(ops.len(), 3);
        // Full-width multiplier: m=4.
        regs.set_r(2, 0x8000_0000);
        let ops = expand_arm(0xE000_0291, &regs).expect("wide mul expands");
        assert_eq!(ops.len(), 5);
        // Plain DP beside the masks stays on its own path.
        assert!(expand_arm_mul(0xE081_0002, &regs).is_none());
    }

    #[test]
    fn arm_mul_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &ARM_MUL_CORPUS,
            false,
            0x4014,
            0x0300_0000,
            &ARM_MUL_REGS,
            ARM_MUL_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Thumb MUL corpus (m from the incoming Rd).
    const THUMB_MUL_CORPUS: [u32; 3] = [
        0x4341, // mul r1, r0
        0x434A, // mul r2, r1
        0x2707, // mov r7, #7
    ];
    const THUMB_MUL_REGS: [(usize, u32); 3] = [(0, 3), (1, 4), (2, 5)];

    #[test]
    fn micro_op_thumb_mul_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_MUL_CORPUS,
                true,
                waitcnt,
                THUMB_MUL_CORPUS.len(),
                0x886A,
                0x0300_0000,
                None,
                &[],
                &THUMB_MUL_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_MUL_CORPUS,
                true,
                waitcnt,
                THUMB_MUL_CORPUS.len(),
                0x886A,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &THUMB_MUL_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn thumb_mul_gate_and_padding() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_cpsr(regs.cpsr() | (1 << 5));
        regs.set_r(1, 4);
        // AND beside the MUL encoding is covered by the ALU-remainder
        // branch (asserted in thumb_alu_rest_shapes_and_gates).
        // MUL m=1 (Rd=4): commit + 1I.
        let ops = expand_thumb(0x4341, &regs).expect("thumb mul expands");
        assert_eq!(ops.len(), 2);
    }

    #[test]
    fn thumb_mul_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &THUMB_MUL_CORPUS,
            true,
            0x0000,
            0x0300_0000,
            &THUMB_MUL_REGS,
            THUMB_MUL_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 9 corpus: Thumb long-BL call and BX return. The call
    // targets 0x10 (high 0xF000, low 0xF806); the subroutine returns
    // via BX LR to the MOV, which then skips over the subroutine.
    const THUMB_BL_CORPUS: [u32; 11] = [
        0xF000, // bl-hi
        0xF806, // bl-lo (target 0x10, LR=0x05)
        0x2707, // mov r7, #7 (return landing)
        0xE005, // b 0x14
        0x2000, // (padding)
        0x2000, // (padding)
        0x2000, // (padding)
        0x2000, // (padding)
        0x2001, // 0x10: mov r0, #1
        0x4770, // bx lr
        0x2102, // 0x14: mov r1, #2
    ];

    #[test]
    fn micro_op_thumb_bl_bx_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_BL_CORPUS,
                true,
                waitcnt,
                6,
                0x886A,
                0x0300_0000,
                None,
                &[],
                &[],
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_BL_CORPUS,
                true,
                waitcnt,
                6,
                0x886A,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &[],
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn thumb_bl_bx_shapes_and_gates() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_cpsr(regs.cpsr() | (1 << 5));
        regs.set_lr(0x0300_0005);
        // BL-high: single LR op. BL-low / BX: refill pair + commit.
        let bl_hi = expand_thumb(0xF000, &regs).expect("bl-hi expands");
        assert_eq!(bl_hi.len(), 1);
        let bl_lo = expand_thumb(0xF806, &regs).expect("bl-lo expands");
        assert_eq!(bl_lo.len(), 3);
        let bx = expand_thumb(0x4770, &regs).expect("bx expands");
        assert_eq!(bx.len(), 3);
        // Hi-reg ADD beside BX is covered by the ALU-remainder branch
        // (asserted in thumb_alu_rest_shapes_and_gates).
    }

    /// BX to an ARM (even) target switches mode; the retire refills
    /// with the ARM width (+8 lead).
    #[test]
    fn bx_switches_mode_like_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write16(0x0300_0000, 0x4708); // bx r1
            bus.write32(0x0200_0100, 0xE3A0_7007); // mov r7, #7 (ARM target)
            cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
            cpu.regs.set_r(1, 0x0200_0100);
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                true,
            )
            .expect("bx must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(b_cpu.regs.pc(), 0x0200_0108);
        assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
        assert!(!b_cpu.regs.cpsr_t());
        assert_eq!(b_cpu.regs.cpsr_t(), a_cpu.regs.cpsr_t());
    }

    #[test]
    fn thumb_bl_bx_tick_parity() {
        let (at, bt, av, bv) = tick_parity(&THUMB_BL_CORPUS, true, 0x0000, 0x0300_0000, &[], 6);
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 10 corpus: Thumb ALU remainder (shifts, ADD/SUB,
    // reg-ALU incl. ROR, hi-reg CMP/ADD).
    const THUMB_ALU_CORPUS: [u32; 10] = [
        0x0041, // lsl r1, r0, #1
        0x0801, // lsr r1, r0, #32
        0x1042, // asr r2, r0, #1
        0x1881, // add r1, r0, r2
        0x1EC1, // sub r1, r0, #3
        0x4001, // and r1, r0
        0x41C1, // ror r1, r0
        0x4580, // cmp r8, r0
        0x4480, // add r8, r0
        0x2707, // mov r7, #7
    ];
    const THUMB_ALU_REGS: [(usize, u32); 5] =
        [(0, 0x8000_0001), (1, 0x100), (2, 4), (3, 0xFF), (8, 0x10)];

    #[test]
    fn micro_op_thumb_alu_rest_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_ALU_CORPUS,
                true,
                waitcnt,
                THUMB_ALU_CORPUS.len(),
                0x886A,
                0x0300_0000,
                None,
                &[],
                &THUMB_ALU_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_ALU_CORPUS,
                true,
                waitcnt,
                THUMB_ALU_CORPUS.len(),
                0x886A,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &THUMB_ALU_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn thumb_alu_rest_shapes_and_gates() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_cpsr(regs.cpsr() | (1 << 5));
        // Single-cycle forms: commit only.
        for instr in [0x0041, 0x1881, 0x4001, 0x4580] {
            let ops = expand_thumb(instr, &regs).expect("alu form expands");
            assert_eq!(ops.len(), 1, "{instr:#06X}");
        }
        // Register shift: commit + 1I. ADD PC: commit + 2I.
        let ops = expand_thumb(0x41C1, &regs).expect("ror expands");
        assert_eq!(ops.len(), 2);
        let ops = expand_thumb(0x4487, &regs).expect("add-pc expands");
        assert_eq!(ops.len(), 3);
        // MUL and BX keep their own branches; SWI stays legacy.
        assert!(expand_thumb_alu_rest(0x4341).is_none());
        assert!(expand_thumb_alu_rest(0x4708).is_none());
        assert!(expand_thumb(0xDF00, &regs).is_none());
    }

    #[test]
    fn thumb_alu_rest_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &THUMB_ALU_CORPUS,
            true,
            0x0000,
            0x0300_0000,
            &THUMB_ALU_REGS,
            THUMB_ALU_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 11 corpus: Thumb ADD SP/PC and SP offset (bus-free).
    const THUMB_SPADD_CORPUS: [u32; 5] = [
        0xA004, // add r0, pc, #0x10
        0xA910, // add r1, sp, #0x40
        0xB00A, // add sp, #0x28
        0xB08A, // sub sp, #0x28
        0x2707, // mov r7, #7
    ];

    #[test]
    fn micro_op_thumb_spadd_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_SPADD_CORPUS,
                true,
                waitcnt,
                THUMB_SPADD_CORPUS.len(),
                0x886A,
                0x0300_0000,
                None,
                &[],
                &[],
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &THUMB_SPADD_CORPUS,
                true,
                waitcnt,
                THUMB_SPADD_CORPUS.len(),
                0x886A,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &[],
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    // Slice 11b: ARM ALU-immediate with imm bit4 set (previously gated
    // to legacy; I==1 routes to DP or Rd==15 PSR-imm, both safe).
    const ARM_IMM4_CORPUS: [u32; 4] = [
        0xE3A0_00FF, // mov r0, #0xFF
        0xE281_101F, // add r1, r1, #0x1F
        0xE252_20F0, // subs r2, r2, #0xF0
        0xE3A0_3005, // mov r3, #5
    ];

    #[test]
    fn micro_op_arm_imm_bit4_matches_legacy() {
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_IMM4_CORPUS,
                false,
                waitcnt,
                ARM_IMM4_CORPUS.len(),
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[],
                &[(1, 1), (2, 0x100)],
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_IMM4_CORPUS,
                false,
                waitcnt,
                ARM_IMM4_CORPUS.len(),
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &[(1, 1), (2, 0x100)],
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn spadd_shapes_and_imm4_gate() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_cpsr(regs.cpsr() | (1 << 5));
        for instr in [0xA004, 0xA910, 0xB00A, 0xB08A] {
            let ops = expand_thumb(instr, &regs).expect("sp-add expands");
            assert_eq!(ops.len(), 1, "{instr:#06X}");
        }
        // Imm bit4 no longer gates the ARM immediate form.
        let ops = expand_arm(0xE3A0_00FF, &regs).expect("bit4 imm expands");
        assert_eq!(ops.len(), 1);
        // MSR-immediate routes to the PSR branch (not DP-imm); MOV
        // with Rd==15 (S=0) expands with refill padding.
        let ops = expand_arm(0xE329_F000, &regs).expect("msr-imm expands");
        assert_eq!(ops.len(), 1);
        let ops = expand_arm(0xE3A0_F005, &regs).expect("mov-pc expands");
        assert_eq!(ops.len(), 3);
    }

    #[test]
    fn spadd_tick_parity() {
        let (at, bt, av, bv) = tick_parity(&THUMB_SPADD_CORPUS, true, 0x0000, 0x0300_0000, &[], 5);
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 12 corpus: ARM single-transfer remainder (register
    // offsets incl. shift, R15 base/dest, halfword-imm R15). Cell A is
    // EWRAM; the [pc] loads read code words as data (deterministic).
    const ARM_SINGLEREST_CORPUS: [u32; 8] = [
        0xE791_0002, // ldr r0, [r1, r2]
        0xE7C1_3004, // strb r3, [r1, r4]
        0xE790_2104, // ldr r2, [r0, r4, lsl #2]
        0xE59F_5000, // ldr r5, [pc]
        0xE581_F004, // str r15, [r1, #4]
        0xE1DF_00B0, // ldrh r0, [pc]
        0xE1C1_F0B0, // strh r15, [r1]
        0xE3A0_7007, // mov r7, #7
    ];
    const ARM_SINGLEREST_MEM: [(u32, u8, u32); 3] = [
        (0x0200_0000, 4, 0xDEAD_0001),
        (0x0200_0004, 4, 0x0200_0020),
        (0x0200_0020, 4, 0xCAFE_BABE),
    ];
    const ARM_SINGLEREST_REGS: [(usize, u32); 5] =
        [(1, 0x0200_0000), (2, 4), (3, 0x1234_5678), (4, 0), (5, 0)];

    #[test]
    fn micro_op_arm_singlerest_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_SINGLEREST_CORPUS,
                false,
                waitcnt,
                ARM_SINGLEREST_CORPUS.len(),
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &ARM_SINGLEREST_MEM,
                &ARM_SINGLEREST_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_SINGLEREST_CORPUS,
                false,
                waitcnt,
                ARM_SINGLEREST_CORPUS.len(),
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &ARM_SINGLEREST_MEM,
                &ARM_SINGLEREST_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn arm_singlerest_shapes() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(1, 0x0200_0000);
        regs.set_r(2, 4);
        // Register offset: [Read, I, I].
        let ops = expand_arm(0xE791_0002, &regs).expect("reg-offset expands");
        assert_eq!(ops.len(), 3);
        // R15 load: +2 refill. R15 store: plain [Write, I].
        let ops = expand_arm(0xE59F_F000, &regs).expect("ldr-pc expands");
        assert_eq!(ops.len(), 5);
        let ops = expand_arm(0xE581_F004, &regs).expect("str-r15 expands");
        assert_eq!(ops.len(), 2);
        let ops = expand_arm(0xE1DF_00B0, &regs).expect("ldrh-pc expands");
        assert_eq!(ops.len(), 3);
    }

    /// Single-transfer memory effects under both engines: stored words
    /// (incl. STR R15 snapshots) as well as registers and totals agree.
    #[test]
    fn arm_singlerest_memory_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            for (i, w) in ARM_SINGLEREST_CORPUS.iter().enumerate() {
                bus.write32(0x0300_0000 + (i as u32) * 4, *w);
            }
            for (addr, width, val) in ARM_SINGLEREST_MEM {
                match width {
                    4 => bus.write32(addr, val),
                    2 => bus.write16(addr, (val & 0xFFFF) as u16),
                    _ => bus.write8(addr, (val & 0xFF) as u8),
                }
            }
            for (r, v) in ARM_SINGLEREST_REGS {
                cpu.regs.set_r(r, v);
            }
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
            let mut total = 0u32;
            let mut queue = std::collections::VecDeque::new();
            for _ in 0..steps {
                let mut acc = 0i64;
                loop {
                    acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                        .expect("corpus must be covered");
                    if queue.is_empty() {
                        break;
                    }
                }
                total += acc.max(1) as u32;
            }
            total
        }
        let (mut a_cpu, mut a_bus) = setup();
        let (mut b_cpu, mut b_bus) = setup();
        let mut ta = 0u32;
        for _ in 0..ARM_SINGLEREST_CORPUS.len() {
            ta += a_cpu.step_legacy(&mut a_bus);
        }
        let tb = drain_micro(&mut b_cpu, &mut b_bus, ARM_SINGLEREST_CORPUS.len());
        assert_eq!((ta, tb), (ta, ta));
        for r in 0..16 {
            assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
        }
        assert_eq!(a_cpu.regs.cpsr(), b_cpu.regs.cpsr());
        for addr in (0x0200_0000..0x0200_0028).step_by(4) {
            assert_eq!(a_bus.read32(addr), b_bus.read32(addr), "cell {addr:#010X}");
        }
    }

    /// LDR pc, [pc]: R15 load diverts; retire flushes at target + lead.
    #[test]
    fn arm_ldr_pc_matches_legacy_and_flushes() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write32(0x0300_0000, 0xE59F_F000); // ldr pc, [pc]
            bus.write32(0x0300_0008, 0x0300_0010);
            bus.write32(0x0300_0010, 0xE3A0_7007); // mov r7, #7 (target)
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                false,
            )
            .expect("ldr-pc must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(b_cpu.regs.pc(), 0x0300_0018); // target + ARM lead
        assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
    }

    #[test]
    fn arm_singlerest_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &ARM_SINGLEREST_CORPUS,
            false,
            0x4014,
            0x0300_0000,
            &ARM_SINGLEREST_REGS,
            ARM_SINGLEREST_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 13 corpus: ARM halfword remainder (signed imm/reg,
    // LDRH reg-offset, STRH reg-offset, LDRSH [pc]). Cell A holds
    // 0x80FF so signedness is observable.
    const ARM_HWREST_CORPUS: [u32; 9] = [
        0xE1D1_00F0, // ldrsh r0, [r1] (imm)
        0xE1D1_00D0, // ldrsb r0, [r1] (imm)
        0xE191_00B2, // ldrh r0, [r1, r2] (reg)
        0xE191_00F2, // ldrsh r0, [r1, r2] (reg)
        0xE191_00D2, // ldrsb r0, [r1, r2] (reg)
        0xE181_00B2, // strh r0, [r1, r2] (reg)
        0xE1DF_00F0, // ldrsh r0, [pc] (R15 base)
        0xE3A0_7007, // mov r7, #7
        0x0000_80FF, // pool halfword (never executed)
    ];
    const ARM_HWREST_MEM: [(u32, u8, u32); 1] = [(0x0200_0000, 4, 0x0000_80FF)];
    const ARM_HWREST_REGS: [(usize, u32); 2] = [(1, 0x0200_0000), (2, 0)];

    #[test]
    fn micro_op_arm_hwrest_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_HWREST_CORPUS,
                false,
                waitcnt,
                8,
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &ARM_HWREST_MEM,
                &ARM_HWREST_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_HWREST_CORPUS,
                false,
                waitcnt,
                8,
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &ARM_HWREST_MEM,
                &ARM_HWREST_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn arm_hwrest_shapes_and_gates() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(1, 0x0200_0000);
        regs.set_r(2, 4);
        // Signed/reg-offset forms expand like the unsigned-imm ones.
        for instr in [
            0xE1D1_00F0,
            0xE1D1_00D0,
            0xE191_00B2,
            0xE191_00F2,
            0xE1DF_00F0,
        ] {
            let ops = expand_arm(instr, &regs).expect("halfword form expands");
            assert_eq!(ops.len(), 3, "{instr:#010X}");
        }
        let ops = expand_arm(0xE181_00B2, &regs).expect("strh-reg expands");
        assert_eq!(ops.len(), 2);
        // Multiply/SWP keep the decoder-first routing (legacy here).
        assert!(expand_arm_single(0xE000_0090, &regs).is_none());
        assert!(expand_arm_single(0xE102_0091, &regs).is_none());
    }

    #[test]
    fn arm_hwrest_memory_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            for (i, w) in ARM_HWREST_CORPUS.iter().enumerate() {
                bus.write32(0x0300_0000 + (i as u32) * 4, *w);
            }
            for (addr, width, val) in ARM_HWREST_MEM {
                match width {
                    4 => bus.write32(addr, val),
                    2 => bus.write16(addr, (val & 0xFFFF) as u16),
                    _ => bus.write8(addr, (val & 0xFF) as u8),
                }
            }
            for (r, v) in ARM_HWREST_REGS {
                cpu.regs.set_r(r, v);
            }
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
            let mut total = 0u32;
            let mut queue = std::collections::VecDeque::new();
            for _ in 0..steps {
                let mut acc = 0i64;
                loop {
                    acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                        .expect("corpus must be covered");
                    if queue.is_empty() {
                        break;
                    }
                }
                total += acc.max(1) as u32;
            }
            total
        }
        let (mut a_cpu, mut a_bus) = setup();
        let (mut b_cpu, mut b_bus) = setup();
        let mut ta = 0u32;
        for _ in 0..8 {
            ta += a_cpu.step_legacy(&mut a_bus);
        }
        let tb = drain_micro(&mut b_cpu, &mut b_bus, 8);
        assert_eq!((ta, tb), (ta, ta));
        for r in 0..16 {
            assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
        }
        assert_eq!(a_cpu.regs.cpsr(), b_cpu.regs.cpsr());
        assert_eq!(a_bus.read32(0x0200_0000), b_bus.read32(0x0200_0000));
    }

    /// ARM LDRSH at an odd BIOS address issues a guarded byte read,
    /// never the Thumb odd-address quirk (guarded halfword + merge).
    /// The latch widths differ (0x78 vs 0x56 below), so this pins the
    /// bus call itself, not just the sign math. The mgba-suite memory
    /// cells pin the same property on hardware (regression: the quirk
    /// broke ROM-OOB/SRAM-mirror/BIOS signed cells).
    #[test]
    fn arm_ldrsh_odd_guarded_byte_read() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write32(0x0300_0000, 0xE1D1_00F0); // ldrsh r0, [r1]
            bus.set_bios_prefetch(0x1234_5678);
            cpu.regs.set_r(1, 0x0000_0001); // odd BIOS address
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        // Guarded byte read: low latch byte, sign-extended.
        assert_eq!(a_cpu.regs.r(0), 0x78);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                false,
            )
            .expect("ldrsh must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(b_cpu.regs.r(0), 0x78);
    }

    #[test]
    fn arm_hwrest_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &ARM_HWREST_CORPUS,
            false,
            0x4014,
            0x0300_0000,
            &ARM_HWREST_REGS,
            8,
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 14 corpus: ARM LDM/STM S-bit forms (run in SYS, where
    // the user bank aliases the current one) plus a plain STM with PC
    // (stored-base quirk + instruction+12 value).
    const ARM_SBIT_CORPUS: [u32; 4] = [
        0xE965_0006, // stmdb r5!, {r1,r2}^
        0xE8F5_4018, // ldmia r5!, {r3,r4}^
        0xE8A6_8042, // stmia r6!, {r1,r6,r15}
        0xE3A0_7007, // mov r7, #7
    ];
    const ARM_SBIT_REGS: [(usize, u32); 5] = [
        (1, 0x1111_1111),
        (2, 0x2222_2222),
        (5, 0x0200_0008),
        (6, 0x0200_0100),
        (7, 0),
    ];

    #[test]
    fn micro_op_arm_sbit_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_SBIT_CORPUS,
                false,
                waitcnt,
                ARM_SBIT_CORPUS.len(),
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[],
                &ARM_SBIT_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_SBIT_CORPUS,
                false,
                waitcnt,
                ARM_SBIT_CORPUS.len(),
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &ARM_SBIT_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    /// S-bit block memory effects in SYS: roundtrip words plus the
    /// STM-PC value slot must agree.
    #[test]
    fn arm_sbit_memory_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            for (i, w) in ARM_SBIT_CORPUS.iter().enumerate() {
                bus.write32(0x0300_0000 + (i as u32) * 4, *w);
            }
            for (r, v) in ARM_SBIT_REGS {
                cpu.regs.set_r(r, v);
            }
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
            let mut total = 0u32;
            let mut queue = std::collections::VecDeque::new();
            for _ in 0..steps {
                let mut acc = 0i64;
                loop {
                    acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                        .expect("corpus must be covered");
                    if queue.is_empty() {
                        break;
                    }
                }
                total += acc.max(1) as u32;
            }
            total
        }
        let (mut a_cpu, mut a_bus) = setup();
        let (mut b_cpu, mut b_bus) = setup();
        let mut ta = 0u32;
        for _ in 0..ARM_SBIT_CORPUS.len() {
            ta += a_cpu.step_legacy(&mut a_bus);
        }
        let tb = drain_micro(&mut b_cpu, &mut b_bus, ARM_SBIT_CORPUS.len());
        assert_eq!((ta, tb), (ta, ta));
        for r in 0..16 {
            assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
        }
        for addr in [
            0x0200_0000,
            0x0200_0004,
            0x0200_0100,
            0x0200_0104,
            0x0200_0108,
        ] {
            assert_eq!(a_bus.read32(addr), b_bus.read32(addr), "cell {addr:#010X}");
        }
    }

    /// User-bank transfer in IRQ mode: STM^ stores the user SP (not
    /// the banked IRQ SP) and LDM^ loads the user LR.
    #[test]
    fn arm_block_s_user_bank_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write32(0x0300_0000, 0xE8C5_2000); // stmia r5, {r13}^
            bus.write32(0x0300_0004, 0xE8D5_4000); // ldmia r5, {r14}^
            cpu.regs.enter_exception(0x12, 0x18, 0x0800_0000, true);
            cpu.regs.set_r(13, 0x0300_7000); // IRQ stack (banked)
            cpu.regs.set_r(5, 0x0200_0000);
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
            let mut total = 0u32;
            let mut queue = std::collections::VecDeque::new();
            for _ in 0..steps {
                let mut acc = 0i64;
                loop {
                    acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                        .expect("corpus must be covered");
                    if queue.is_empty() {
                        break;
                    }
                }
                total += acc.max(1) as u32;
            }
            total
        }
        let (mut a_cpu, mut a_bus) = setup();
        let mut ta = 0u32;
        for _ in 0..2 {
            ta += a_cpu.step_legacy(&mut a_bus);
        }
        // The user SP (not the IRQ SP) hits memory.
        assert_eq!(a_bus.read32(0x0200_0000), 0x0300_7F00);
        let (mut b_cpu, mut b_bus) = setup();
        let tb = drain_micro(&mut b_cpu, &mut b_bus, 2);
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(b_bus.read32(0x0200_0000), 0x0300_7F00);
        for r in 0..16 {
            assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
        }
        // User LR received the cell (bank-to-bank through memory).
        assert_eq!(a_cpu.regs.user_r(14), 0x0300_7F00);
        assert_eq!(b_cpu.regs.user_r(14), 0x0300_7F00);
    }

    /// LDM^ with PC in IRQ mode: CPSR restores from SPSR (back to SYS)
    /// and PC loads; retire refills at the target.
    #[test]
    fn arm_ldm_s_pc_restore_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write32(0x0300_0000, 0xE8D5_8001); // ldmia r5, {r0,pc}^
            bus.write32(0x0200_0000, 0);
            bus.write32(0x0200_0004, 0x0300_0010);
            bus.write32(0x0300_0010, 0xE3A0_7007); // mov r7, #7 (target)
            cpu.regs.enter_exception(0x12, 0x18, 0x0800_0000, true);
            // spsr_irq already holds pre-exception SYS cpsr (0x1F).
            cpu.regs.set_r(5, 0x0200_0000);
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        assert_eq!(a_cpu.regs.cpsr_mode(), 0x1F);
        assert_eq!(a_cpu.regs.pc(), 0x0300_0018);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                false,
            )
            .expect("ldm^-pc must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(b_cpu.regs.cpsr_mode(), 0x1F);
        assert_eq!(b_cpu.regs.pc(), 0x0300_0018);
        assert_eq!(b_cpu.regs.r(0), 0);
        assert_eq!(b_cpu.regs.r(0), a_cpu.regs.r(0));
    }

    #[test]
    fn arm_sbit_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &ARM_SBIT_CORPUS,
            false,
            0x4014,
            0x0300_0000,
            &ARM_SBIT_REGS,
            ARM_SBIT_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 15 corpus: ARM SWP/SWPB roundtrip through an EWRAM cell.
    const ARM_SWP_CORPUS: [u32; 3] = [
        0xE102_0091, // swp r0, r1, [r2]
        0xE142_0091, // swpb r0, r1, [r2]
        0xE3A0_7007, // mov r7, #7
    ];

    #[test]
    fn micro_op_arm_swp_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_SWP_CORPUS,
                false,
                waitcnt,
                ARM_SWP_CORPUS.len(),
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[(0x0200_0000, 4, 0x5555_5555)],
                &[(1, 0xAAAA_AAAA), (2, 0x0200_0000)],
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_SWP_CORPUS,
                false,
                waitcnt,
                ARM_SWP_CORPUS.len(),
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &[(0x0200_0000, 4, 0x5555_5555)],
                &[(1, 0xAAAA_AAAA), (2, 0x0200_0000)],
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn arm_swp_shapes() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(2, 0x0200_0000);
        // Atomic commit + 3 trailing = 4.
        let ops = expand_arm(0xE102_0091, &regs).expect("swp expands");
        assert_eq!(ops.len(), 4);
        let ops = expand_arm(0xE142_0091, &regs).expect("swpb expands");
        assert_eq!(ops.len(), 4);
        // The single-transfer branch must not claim the SWP mask.
        assert!(expand_arm_single(0xE102_0091, &regs).is_none());
    }

    #[test]
    fn arm_swp_memory_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            for (i, w) in ARM_SWP_CORPUS.iter().enumerate() {
                bus.write32(0x0300_0000 + (i as u32) * 4, *w);
            }
            bus.write32(0x0200_0000, 0x5555_5555);
            cpu.regs.set_r(1, 0xAAAA_AAAA);
            cpu.regs.set_r(2, 0x0200_0000);
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
            let mut total = 0u32;
            let mut queue = std::collections::VecDeque::new();
            for _ in 0..steps {
                let mut acc = 0i64;
                loop {
                    acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                        .expect("corpus must be covered");
                    if queue.is_empty() {
                        break;
                    }
                }
                total += acc.max(1) as u32;
            }
            total
        }
        let (mut a_cpu, mut a_bus) = setup();
        let (mut b_cpu, mut b_bus) = setup();
        let mut ta = 0u32;
        for _ in 0..ARM_SWP_CORPUS.len() {
            ta += a_cpu.step_legacy(&mut a_bus);
        }
        let tb = drain_micro(&mut b_cpu, &mut b_bus, ARM_SWP_CORPUS.len());
        assert_eq!((ta, tb), (ta, ta));
        for r in 0..16 {
            assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
        }
        // SWP exchanged, SWPB rewrote the low byte with itself.
        assert_eq!(a_bus.read32(0x0200_0000), 0xAAAA_AAAA);
        assert_eq!(b_bus.read32(0x0200_0000), 0xAAAA_AAAA);
    }

    #[test]
    fn arm_swp_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &ARM_SWP_CORPUS,
            false,
            0x4014,
            0x0300_0000,
            &[(1, 0xAAAA_AAAA), (2, 0x0200_0000)],
            ARM_SWP_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 16 corpus: ARM MRS/MSR + BX over a relative target
    // (ADD pc keeps the corpus position-independent for the ROM
    // variant). MSR sets N via r0; TST observes it; BX skips two.
    const ARM_PSRBX_CORPUS: [u32; 10] = [
        0xE10F_1000, // mrs r1, cpsr
        0xE129_F000, // msr cpsr_f, r0
        0xE111_0000, // tst r1, r0
        0xE28F_300C, // add r3, pc, #12
        0xE12F_FF13, // bx r3
        0xE3A0_4005, // mov r4, #5 (skipped)
        0xE3A0_5006, // mov r5, #6 (skipped)
        0xE3A0_2009, // mov r2, #9 (skipped)
        0xE3A0_6007, // mov r6, #7 (BX landing)
        0xE3A0_7008, // mov r7, #8
    ];
    const ARM_PSRBX_REGS: [(usize, u32); 2] = [(0, 0x8000_0000), (3, 0)];

    #[test]
    fn micro_op_arm_psrbx_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_PSRBX_CORPUS,
                false,
                waitcnt,
                7,
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[],
                &ARM_PSRBX_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_PSRBX_CORPUS,
                false,
                waitcnt,
                7,
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &ARM_PSRBX_REGS,
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn arm_psrbx_shapes_and_gates() {
        let regs = CpuRegisters::post_bios();
        // PSR forms: single commit op.
        for instr in [0xE10F_1000, 0xE129_F000, 0xE32B_F000] {
            let ops = expand_arm(instr, &regs).expect("psr expands");
            assert_eq!(ops.len(), 1, "{instr:#010X}");
        }
        // ARM BX: refill pair + commit.
        let ops = expand_arm(0xE12F_FF13, &regs).expect("arm bx expands");
        assert_eq!(ops.len(), 3);
        // DP-imm with Rn==PC reads the execute-stage PC (bus-free).
        let ops = expand_arm(0xE28F_300C, &regs).expect("add-pc expands");
        assert_eq!(ops.len(), 1);
    }

    /// ARM BX to Thumb: mode switches at execution; retire refills
    /// with halfword width.
    #[test]
    fn arm_bx_to_thumb_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write32(0x0200_0000, 0xE12F_FF10); // bx r0
            bus.write16(0x0300_0040, 0x2707); // mov r7, #7 (Thumb target)
            cpu.regs.set_r(0, 0x0300_0041); // bit0 set: Thumb
            cpu.regs.set_pc(0x0200_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                false,
            )
            .expect("arm bx must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(b_cpu.regs.pc(), 0x0300_0044); // target + Thumb lead
        assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
        assert!(b_cpu.regs.cpsr_t());
        assert_eq!(b_cpu.regs.cpsr_t(), a_cpu.regs.cpsr_t());
    }

    #[test]
    fn arm_psrbx_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &ARM_PSRBX_CORPUS,
            false,
            0x4014,
            0x0300_0000,
            &ARM_PSRBX_REGS,
            7,
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 17 corpus: ARM DP-imm full set (logical, reverse and
    // carry arithmetic, flag-only forms, rotated S=0). r1 carries
    // 0xFF00FF00; the SUBS seeds C=1 for the SBC/RSC chain.
    const ARM_ALUFULL_CORPUS: [u32; 16] = [
        0xE201_00FF, // and r0, r1, #0xFF
        0xE221_00FF, // eor r0, r1, #0xFF
        0xE251_0001, // subs r0, r1, #1 (C=1)
        0xE2C1_0000, // sbc r0, r1, #0
        0xE2E1_0000, // rsc r0, r1, #0
        0xE261_0001, // rsb r0, r1, #1
        0xE2A1_0001, // adc r0, r1, #1
        0xE311_00FF, // tst r1, #0xFF
        0xE111_0000, // teq r1, r0 (S=0: flags untouched)
        0xE371_0001, // cmn r1, #1
        0xE381_0001, // orr r0, r1, #1
        0xE3C1_00FF, // bic r0, r1, #0xFF
        0xE3E0_0000, // mvn r0, #0
        0xE201_04FF, // and r0, r1, #0xFF000000 (rotated, S=0)
        0xE3A0_3005, // mov r3, #5
        0xE3A0_7007, // mov r7, #7
    ];

    #[test]
    fn micro_op_arm_alufull_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_ALUFULL_CORPUS,
                false,
                waitcnt,
                ARM_ALUFULL_CORPUS.len(),
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[],
                &[(1, 0xFF00_FF00)],
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_ALUFULL_CORPUS,
                false,
                waitcnt,
                ARM_ALUFULL_CORPUS.len(),
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &[(1, 0xFF00_FF00)],
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn arm_alufull_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &ARM_ALUFULL_CORPUS,
            false,
            0x4014,
            0x0300_0000,
            &[(1, 0xFF00_FF00)],
            ARM_ALUFULL_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    // Slice 18 corpus: S with rotated immediates (shifter carry).
    const ARM_SROT_CORPUS: [u32; 5] = [
        0xE211_04FF, // ands r0, r1, #0xFF000000 (C=1)
        0xE291_04FF, // adds r0, r1, #0xFF000000
        0xE3B0_04FF, // movs r0, #0xFF000000 (N=1, C=1)
        0xE3A0_3005, // mov r3, #5
        0xE3A0_7007, // mov r7, #7
    ];

    #[test]
    fn micro_op_arm_srot_matches_legacy() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_SROT_CORPUS,
                false,
                waitcnt,
                ARM_SROT_CORPUS.len(),
                0xE1DD_20B0,
                0x0300_0000,
                None,
                &[],
                &[(1, 1)],
            );
            assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
            assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        }
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs_equal, follow) = differential(
                &ARM_SROT_CORPUS,
                false,
                waitcnt,
                ARM_SROT_CORPUS.len(),
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &[],
                &[(1, 1)],
            );
            assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
            assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
            assert!(follow, "rom waitcnt={waitcnt:#06x}");
        }
    }

    /// SUBS pc, lr, #4 in IRQ mode (immediate form): CPSR restores
    /// from SPSR (to SVC here) and PC loads; retire refills.
    #[test]
    fn arm_subs_pc_restore_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write32(0x0300_0000, 0xE25E_F004); // subs pc, lr, #4
            bus.write32(0x0300_0010, 0xE3A0_7007); // mov r7, #7 (target)
            cpu.regs.enter_exception(0x12, 0x18, 0x0800_0000, true);
            cpu.regs.set_spsr(0x13); // return to SVC on restore
            cpu.regs.set_lr(0x0300_0014);
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        assert_eq!(a_cpu.regs.cpsr_mode(), 0x13);
        assert_eq!(a_cpu.regs.pc(), 0x0300_0018);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                false,
            )
            .expect("subs-pc must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(b_cpu.regs.cpsr_mode(), 0x13);
        assert_eq!(b_cpu.regs.pc(), 0x0300_0018);
    }

    /// MOVS pc, lr in IRQ mode (register form, delegate path): same
    /// restore + load contract through CommitDpReg.
    #[test]
    fn arm_movs_pc_restore_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write32(0x0300_0000, 0xE1B0_F00E); // movs pc, lr
            bus.write32(0x0300_0010, 0xE3A0_7007); // mov r7, #7 (target)
            cpu.regs.enter_exception(0x12, 0x18, 0x0800_0000, true);
            cpu.regs.set_spsr(0x13); // return to SVC on restore
            cpu.regs.set_lr(0x0300_0010);
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        assert_eq!(a_cpu.regs.cpsr_mode(), 0x13);
        assert_eq!(a_cpu.regs.pc(), 0x0300_0018);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                false,
            )
            .expect("movs-pc must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(b_cpu.regs.cpsr_mode(), 0x13);
        assert_eq!(b_cpu.regs.pc(), 0x0300_0018);
    }

    /// SUBS pc, lr, #0 in SYS mode (no SPSR): PC writes and flags
    /// update, no restore.
    #[test]
    fn arm_subs_pc_sys_matches_legacy() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write32(0x0300_0000, 0xE25E_F000); // subs pc, lr, #0
            bus.write32(0x0300_0010, 0xE3A0_7007); // mov r7, #7 (target)
            cpu.regs.set_lr(0x0300_0010);
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                false,
            )
            .expect("subs-pc must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(b_cpu.regs.pc(), 0x0300_0018);
        assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
        assert_eq!(b_cpu.regs.cpsr(), a_cpu.regs.cpsr());
    }

    /// Flag-only Rd==15 with S in IRQ mode: CPSR restores, PC advances
    /// without a flush (legacy refill count, no latch).
    #[test]
    fn arm_tst_pc_restore_no_flush() {
        fn setup() -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write32(0x0300_0000, 0xE111_F00E); // tst pc, lr (S=1)
            bus.write32(0x0300_0004, 0xE3A0_7007); // mov r7, #7
            cpu.regs.enter_exception(0x12, 0x18, 0x0800_0000, true);
            cpu.regs.set_spsr(0x1F); // restore SYS (ARM)
            cpu.regs.set_lr(0x0300_0100);
            cpu.regs.set_pc(0x0300_0000);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        let (mut a_cpu, mut a_bus) = setup();
        let ta = a_cpu.step_legacy(&mut a_bus);
        assert_eq!(a_cpu.regs.cpsr_mode(), 0x1F);
        let (mut b_cpu, mut b_bus) = setup();
        let mut queue = std::collections::VecDeque::new();
        let mut acc = 0i64;
        loop {
            acc += step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut queue,
                false,
            )
            .expect("tst-pc must be covered");
            if queue.is_empty() {
                break;
            }
        }
        let tb = acc.max(1) as u32;
        assert_eq!((ta, tb), (ta, ta));
        assert_eq!(b_cpu.regs.cpsr_mode(), 0x1F);
        // No flush: sequential advance past the restored stream.
        assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
    }

    #[test]
    fn arm_srot_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &ARM_SROT_CORPUS,
            false,
            0x4014,
            0x0300_0000,
            &[(1, 1)],
            ARM_SROT_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    /// Differential grid over S+rotated DP-immediate space: all 16
    /// opcodes, rotation fields, low immediates and base registers.
    /// Pins the shifter-carry snapshot against the live CPSR carry
    /// (ADC/SBC/RSC read the latter even under rotation).
    #[test]
    fn arm_dpimm_srot_grid() {
        let mut bad = 0;
        for opcode in 0..16u32 {
            for rot in [1u32, 2, 4, 7, 15] {
                for imm8 in [0u32, 1, 0xFF, 0x80] {
                    for rn in [0usize, 1] {
                        let instr = 0xE000_0000
                            | (opcode << 21)
                            | (1 << 20)
                            | ((rn as u32) << 16)
                            | (rot << 8)
                            | imm8
                            | (1 << 25);
                        let code = [instr, 0xE3A0_7007];
                        let (ta, tb, regs, follow) = differential(
                            &code,
                            false,
                            0x0000,
                            1,
                            0xE1DD_20B0,
                            0x0300_0000,
                            None,
                            &[],
                            &[(0, 0x1234_5678), (1, 0xFF00_FF00)],
                        );
                        if ta != tb || !regs || !follow {
                            eprintln!(
                                "DIVERGE op={:#X} rot={} imm={:#X} rn={}: ta={} tb={} regs={} follow={}",
                                opcode, rot, imm8, rn, ta, tb, regs, follow
                            );
                            bad += 1;
                            if bad > 8 {
                                panic!("too many divergences");
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(bad, 0);
    }

    /// Coverage manifest: every instruction class is either expanded
    /// or intentionally legacy. Intentional-legacy (None): empty block
    /// lists (quirk paths), UND/SWI (exceptions), coprocessor (UND on
    /// GBA). Everything else in both ISAs must expand; add new classes
    /// here when extending coverage.
    #[test]
    fn coverage_manifest() {
        let regs = CpuRegisters::post_bios();
        // Intentionally legacy ARM: empty lists, SWI, coprocessor UND.
        for instr in [
            0xE8A0_0000, // empty stmia
            0xE8F0_0000, // empty ldmia^
            0xEF00_0000, // swi
            0xEE00_0010, // coprocessor (UND)
            0xEC00_0000, // coprocessor (UND)
        ] {
            assert!(expand_arm(instr, &regs).is_none(), "{instr:#010X}");
        }
        // Intentionally legacy Thumb: empty lists, UND, SWI.
        let mut tregs = CpuRegisters::post_bios();
        tregs.set_cpsr(tregs.cpsr() | (1 << 5));
        for instr in [
            0xB400, // empty push
            0xBC00, // empty pop
            0xC000, // empty stmia
            0xC800, // empty ldmia
            0xDE00, // undefined
            0xDF00, // swi
        ] {
            assert!(expand_thumb(instr, &tregs).is_none(), "{instr:#06X}");
        }
        // Covered ARM representatives (one per class/form).
        let mut aregs = CpuRegisters::post_bios();
        aregs.set_r(0, 0x0200_0000);
        aregs.set_r(1, 0xFF00_FF00);
        aregs.set_r(2, 4);
        aregs.set_r(5, 0x0200_0008);
        for instr in [
            0xE3A0_0001, // dp-imm mov
            0xE211_04FF, // dp-imm S+rot
            0xE3A0_F005, // dp-imm Rd==15
            0xE081_0002, // dp-reg
            0xE1B0_0213, // dp-reg shift
            0xE10F_1000, // mrs
            0xE129_F000, // msr-reg
            0xE32B_F000, // msr-imm
            0xE12F_FF11, // bx
            0xE000_0291, // mul
            0xE022_0391, // mla
            0xE083_2190, // umull
            0xE102_0091, // swp
            0xE581_0004, // str imm
            0xE791_0002, // ldr reg-offset
            0xE59F_5000, // ldr literal (rn==pc)
            0xE59F_F000, // ldr pc (Rd==15)
            0xE1D1_00B0, // ldrh imm (S:H == 01)
            0xE1D1_00F0, // ldrsh imm (S:H == 11)
            0xE1D1_00D0, // ldrsb imm (S:H == 10)
            0xE191_00B2, // ldrh reg-offset
            0xE7C1_3004, // strb reg-offset
            0xE1D1_00F0, // ldrsh imm
            0xE191_00B2, // ldrh reg-offset
            0xE8A0_0006, // stmia
            0xE8B5_0018, // ldmia
            0xE8F5_4018, // ldmia^ (S bit)
            0xE890_8000, // ldmia pc
            0xEA00_0001, // b
            0xEB00_0001, // bl (link)
        ] {
            assert!(expand_arm(instr, &aregs).is_some(), "{instr:#010X}");
        }
        // Covered Thumb representatives (one per class/form).
        for instr in [
            0x0041, // lsl imm
            0x1881, // add reg
            0x2001, // mov imm
            0x4341, // mul
            0x4001, // and reg
            0x41C1, // ror reg (+1I)
            0x4400, // add hi-reg
            0x4487, // add pc (+2)
            0x4580, // cmp hi-reg
            0x4770, // bx
            0x4801, // ldr literal
            0x5088, // str reg-offset
            0x5E8F, // ldrsh reg-offset
            0x6088, // str imm-offset
            0x8090, // strh
            0x9010, // str sp-relative
            0x9D10, // ldr sp-relative
            0xA004, // add pc
            0xB00A, // add sp
            0xB40F, // push
            0xBCF0, // pop
            0xBD02, // pop pc
            0xC10F, // stmia
            0xCB18, // ldmia (base in list)
            0xD001, // cond branch
            0xE001, // b
            0xF000, // bl high
            0xF806, // bl low
        ] {
            assert!(expand_thumb(instr, &tregs).is_some(), "{instr:#06X}");
        }
    }

    #[test]
    fn micro_op_rom_code_matches_legacy() {
        for waitcnt in [0x0000u16, 0x4010, 0x4014] {
            let (ta, tb, regs, follow) = differential(
                &ARM_ROM_CORPUS,
                false,
                waitcnt,
                9,
                0xE1DD_20B0,
                0x0800_0100,
                Some(rom_cart()),
                &[(0x0200_0008, 4, 0xAABB_CCDD)],
                &[(0, 0x1234_5678)],
            );
            assert_eq!((ta, tb), (ta, ta), "waitcnt={waitcnt:#06x}");
            assert!(regs, "waitcnt={waitcnt:#06x}");
            assert!(follow, "waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn micro_op_flag_carryover() {
        // V set by an overflowing ADD must survive a following MOV
        // (logical class preserves V in both modes; the legacy Thumb
        // MOV-imm additionally forces N=0). Caught by differential:
        // legacy keeps V=1, a clobbering engine would read V=0.
        let (ta, tb, regs, follow) = differential(
            &[0x3001u32, 0x2102], // add r0, #1 (0x7FFFFFFF -> V=1); mov r1, #2
            true,
            0x0000,
            2,
            0x886A,
            0x0300_0000,
            None,
            &[],
            &[(0, 0x7FFF_FFFF)],
        );
        assert_eq!((ta, tb), (ta, ta));
        assert!(regs);
        assert!(follow);
    }

    /// Tick parity between legacy and micro drain rhythms, ticking the bus per elapsed tick with TM0 running.
    /// Returns (legacy_ticks, micro_ticks, legacy_tm0, micro_tm0).
    /// Divergence here shifts every timer-measured cell, pinning the system drain loop.
    fn tick_parity(
        code: &[u32],
        thumb: bool,
        waitcnt: u16,
        code_base: u32,
        reg_init: &[(usize, u32)],
        steps: usize,
    ) -> (u32, u32, u32, u32) {
        fn setup(
            code: &[u32],
            thumb: bool,
            waitcnt: u16,
            code_base: u32,
            reg_init: &[(usize, u32)],
        ) -> (GbaCpu, GbaMemoryBus) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write16(0x04000204, waitcnt);
            let stride = if thumb { 2 } else { 4 };
            for (i, w) in code.iter().enumerate() {
                let addr = code_base + (i as u32) * stride as u32;
                if thumb {
                    bus.write16(addr, (w & 0xFFFF) as u16);
                } else {
                    bus.write32(addr, *w);
                }
            }
            for (r, v) in reg_init {
                cpu.regs.set_r(*r, *v);
            }
            if thumb {
                cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
            }
            cpu.regs.set_pc(code_base);
            fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
            bus.take_access_wait_cycles();
            // TM0 free-run /1 from 0 (the suite START shape, minus the
            // control write which the corpus itself performs if needed).
            bus.write32(0x0400_0100, 0x0080_0000);
            bus.take_access_wait_cycles();
            (cpu, bus)
        }
        // Legacy rhythm: whole step, then one tick per returned cycle.
        let (mut a_cpu, mut a_bus) = setup(code, thumb, waitcnt, code_base, reg_init);
        let mut a_ticks = 0u32;
        for _ in 0..steps {
            let c = a_cpu.step_legacy(&mut a_bus).max(1);
            for _ in 0..c {
                a_bus.tick();
                a_ticks += 1;
            }
        }
        let a_tm0 = a_bus.read16(0x0400_0100);
        // Micro rhythm: mirror of the system.rs drain loop, one tick at a
        // time until every corpus instruction has retired.
        let (mut b_cpu, mut b_bus) = setup(code, thumb, waitcnt, code_base, reg_init);
        let mut b_queue = std::collections::VecDeque::new();
        let mut b_ticks = 0u32;
        let mut retired = 0usize;
        while retired < steps {
            let mut acc = 0i64;
            loop {
                let c = step_op(
                    &mut b_cpu.regs,
                    &mut b_bus,
                    &mut b_cpu.pipeline,
                    &mut b_queue,
                    thumb,
                )
                .expect("corpus must be covered");
                acc += c;
                if b_queue.is_empty() {
                    retired += 1;
                    break;
                }
                if acc >= 1 {
                    break;
                }
            }
            let spend = acc.max(1) as u32;
            for _ in 0..spend {
                b_bus.tick();
                b_ticks += 1;
            }
        }
        let b_tm0 = b_bus.read16(0x0400_0100);
        (a_ticks, b_ticks, u32::from(a_tm0), u32::from(b_tm0))
    }

    #[test]
    fn micro_op_tick_parity_timer_span() {
        // Calibration shape: TM0 start already running (setup), one
        // payload read, one control write — the measured span must match
        // between rhythms (IWRAM code, Thumb).
        let code = [
            0x9802u32, // ldr r0, [sp, #8] (sp data, covered)
            0x9003,    // str r0, [sp, #12]
        ];
        let (at, bt, av, bv) = tick_parity(&code, true, 0x0000, 0x0300_0000, &[], code.len());
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }

    #[test]
    fn conditional_branch_loop_tick_parity() {
        let arm = [
            0xE3A0_0000,
            0xE280_0001,
            0xE350_0003,
            0x1AFF_FFFC,
            0xE3A0_1007,
        ];
        let thumb = [0x2000, 0x3001, 0x2803, 0xD1FC, 0x2107];

        let (at, bt, av, bv) = tick_parity(&arm, false, 0x4014, 0x0300_0000, &[], 11);
        assert_eq!((at, bt), (at, at), "ARM ticks diverge");
        assert_eq!((av, bv), (av, av), "ARM timer span diverges");

        let (at, bt, av, bv) = tick_parity(&thumb, true, 0x4014, 0x0300_0000, &[], 11);
        assert_eq!((at, bt), (at, at), "Thumb ticks diverge");
        assert_eq!((av, bv), (av, av), "Thumb timer span diverges");
    }

    #[test]
    fn register_offset_load_store_tick_parity() {
        let (at, bt, av, bv) = tick_parity(
            &THUMB_REG_LS_CORPUS,
            true,
            0x4014,
            0x0300_0000,
            &[(0, 0x0000_80FF), (1, 0x0300_0100), (2, 4)],
            THUMB_REG_LS_CORPUS.len(),
        );
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }
}
