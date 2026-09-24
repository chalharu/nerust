//! Micro-op expansion for covered instructions, interpreted one step at a time.
//! Decode stays instruction-atomic; uncovered instructions return `None` (legacy path).
//! Totals match the legacy step per class, pinned by the differential tests below.

use std::collections::VecDeque;

use crate::cpu::arm_opcodes::block_transfer::start_address;
use crate::cpu::arm_opcodes::helpers::{
    barrel_shift, barrel_shift_register, condition_passed, update_nz,
};
use crate::cpu::arm_opcodes::multiply::{
    multiplier_cycles, multiplier_cycles_long, multiply_64, multiply_carry_hi, multiply_carry_lo,
    multiply_tick_full, register_pair,
};
use crate::cpu::arm_opcodes::psr_transfer::is_psr_transfer;
use crate::cpu_pipeline::fill_pipeline;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// One sub-instruction effect; effects land at execute-stage points with
/// the legacy bus-call order (access, then fetch-stream-break).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicroOp {
    Internal,
    CommitAlu(AluEffect),
    /// ARM data-processing (register form): raw word, applied natively
    /// with its base returned separately (trailing `Internal`s pad it).
    /// Costs +1 here; no bus access, so only the cycle split is new.
    CommitDpReg(u32),
    /// Multiply commit: raw word plus mode. ARM short/long run the
    /// native apply; Thumb MUL mirrors the decoder preamble
    /// (fetch break + P-ON erase) plus the native ALU. Costs +1; the
    /// m internal ticks become trailing `Internal` event points.
    CommitMul(MulEffect),
    /// SWP commit: raw word, applied atomically by the native apply
    /// (the HW bus lock forbids DMA between the read and the write, so
    /// the pair never splits across ops). Costs +1; the remaining base
    /// becomes trailing `Internal` event points.
    CommitSwp(u32),
    /// ARM MRS/MSR: raw word, applied natively (pure
    /// registers, no bus; T-bit preserving, so no mode-switch flush).
    /// Single op carrying the whole base of 1.
    CommitPsr(u32),
    /// Thumb ALU remainder (move-shifted, ADD/SUB, reg-ALU, hi-reg):
    /// raw halfword; the apply step runs the native implementation
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
    /// SWI trap: BIOS HLE number. The apply step runs the HLE
    /// dispatcher and carries its full charge (SVC-vector entry on
    /// Unsupported), mirroring the legacy SWI handlers exactly.
    TrapSwi(u8),
    /// Undefined-instruction trap: exception entry, mirroring the
    /// legacy UND handlers exactly (2S+1I+1N = 4 in both states).
    TrapUnd,
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
    /// Empty-list transfer (ARM LDM/STM with Rlist=0, Thumb PUSH/POP
    /// with no registers): the single PC word. Costs +1 like a block
    /// word; trailing `Internal`s pad the legacy base.
    BlockEmpty(BlockEmptyEffect),
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
    /// First word address, for the instruction-scoped fetch-stream break
    /// (open-bus blocks pre-pay nothing, mapped blocks break).
    pub first_addr: u32,
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

/// One empty-list block word (ARM LDM/STM Rlist=0, Thumb PUSH/POP with
/// no registers): the single transferred PC word. Address and store
/// value are snapshotted at expansion (frozen pre-instruction state,
/// like `BlockWord`); the access runs at execution through the same
/// bus calls as the legacy empty path, including its batch/no-batch
/// shape (Thumb batches, ARM does not) and sequential-touch shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockEmptyEffect {
    pub load: bool,
    pub addr: u32,
    /// ARM writeback (W=1): `(base_reg, base +/- 0x40)` via `set_r`.
    /// None when W=0 (ARM) or for Thumb (SP goes through `BlockEnd.sp`,
    /// which uses `set_sp` like the legacy empty path).
    pub writeback_reg: Option<(usize, u32)>,
    /// Store word for STM/PUSH (PC + 4 ARM / + 2 Thumb), snapshotted
    /// at expansion.
    pub store_value: u32,
    /// ARM LDM^ loading PC: restore CPSR from SPSR. Evaluated at
    /// execution like the `BlockWord` path.
    pub restore_cpsr: bool,
    /// Touch `data_sequential` before the access (Thumb empty paths
    /// set it false; ARM empty paths leave it alone, like legacy).
    pub reset_sequential: bool,
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
        // NV: never executes (ARM ARM); the legacy path retires it as
        // a 1S NOP, so expansion is a single Internal.
        return Some(vec![MicroOp::Internal]);
    }
    // SWI: class 111 with bit 24 set (any condition; the trap
    // number is in bits 23-16 for HLE). Failed conditions retire as
    // [Internal] via the wrapper below, like the legacy cond check.
    if (instr >> 25) & 0b111 == 0b111 && (instr >> 24) & 1 == 1 {
        return Some(vec![MicroOp::TrapSwi(((instr >> 16) & 0xFF) as u8)]);
    }
    // UND: coprocessor data class (110) and class 111 without the SWI
    // bit. The GBA has no coprocessor, so all such encodings trap.
    if (instr >> 25) & 0b111 == 0b110 || ((instr >> 25) & 0b111 == 0b111 && (instr >> 24) & 1 == 0)
    {
        return Some(vec![MicroOp::TrapUnd]);
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
/// match by construction (commit runs the native apply).
pub(crate) fn expand_arm_dp_reg(instr: u32, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
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
/// MLA +1I; long 1S+mI+1I, accumulate +1I); the commit runs the native
/// apply, so only the internal-tick split is new.
pub(crate) fn expand_arm_mul(instr: u32, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
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
/// PC+4 store on STM). No batch framing, mirroring the legacy empty
/// path; the op carries +1 with trailing Internals to the base
/// (LDM+PC 5, STM 2).
fn expand_arm_block_empty(spec: BlockEmptySpec, regs: &CpuRegisters) -> Vec<MicroOp> {
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
    let mut ops = vec![MicroOp::BlockEmpty(BlockEmptyEffect {
        load: spec.load,
        addr,
        writeback_reg: writeback,
        store_value: regs.pc().wrapping_add(4),
        restore_cpsr: spec.s,
        reset_sequential: false,
    })];
    ops.extend(vec![MicroOp::Internal; if spec.load { 4 } else { 1 }]);
    ops
}

/// STM store-word snapshot at expansion (frozen registers and mode):
/// user-bank reads, the stored-base quirk, and r15 as instruction+12
/// (legacy `store_register`).
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

/// Pad the legacy `transfer_cycles` base (LDM 2+n, LDM+PC 4+n,
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
    let base = regs.r(rn);
    if list == 0 {
        return Some(expand_arm_block_empty(
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
        ));
    }
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
        let store_value = block_store_value(load, stored_base, user_bank, regs, *reg);
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
    let writeback = block_writeback(writeback_flag, load, list, rn, final_addr);
    // Post-LDM^ conflict, armed exactly like the legacy tail (the PC
    // case never sets user_bank, so expansion-time mode is exact).
    let conflict = load && user_bank && !matches!(regs.cpsr_mode(), 0x10 | 0x1F);
    ops.push(MicroOp::BlockEnd(BlockEndEffect {
        sp: None,
        writeback,
        ldm_conflict: conflict,
        first_addr: start,
    }));
    // Pad the legacy `transfer_cycles` base (LDM 2+n, LDM+PC 4+n,
    // STM 1+n; words already carry +1 each).
    let trailing = block_trailing(load, list);
    ops.extend(vec![MicroOp::Internal; trailing]);
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
pub(crate) fn expand_arm_single(instr: u32, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    let dec = SingleDecoded {
        l: (instr >> 20) & 1 == 1,
        pre_indexed: (instr >> 24) & 1 == 1,
        writeback: (instr >> 24) & 1 == 0 || (instr >> 21) & 1 == 1,
        rn: ((instr >> 16) & 0xF) as usize,
        rd: ((instr >> 12) & 0xF) as usize,
        subtract: (instr >> 23) & 1 == 0,
        // STR of R15 stores instruction+12 (legacy `single_transfer`).
        store_value: single_store_value(instr, regs),
    };
    // Word/byte class (bits27-26 == 01), immediate or register offset.
    if (instr >> 26) & 0x3 == 0b01 {
        return single_word(instr, regs, &dec);
    }
    // Halfword class (bits27-25 == 000, bit7+bit4 set): immediate and
    // register offsets, all S:H shapes (unsigned half, signed byte /
    // half; S:H == 00 behaves as halfword like the legacy handler).
    // Multiply/SWP/PSR/BX patterns carry the tag too, so the decoder
    // exclusions are mirrored (decode tests them first).
    if (instr >> 25) & 0x7 == 0 && (instr & 0x00000090) == 0x00000090 {
        return single_half(instr, regs, &dec);
    }
    None
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

fn single_store_value(instr: u32, regs: &CpuRegisters) -> Option<u32> {
    let l = (instr >> 20) & 1 == 1;
    let rd = ((instr >> 12) & 0xF) as usize;
    if !l && rd == 15 {
        Some(regs.r(15).wrapping_add(4))
    } else {
        None
    }
}

/// Pad the legacy base: loads 3 (5 for R15), stores 2.
fn single_ops(l: bool, rd: usize, acc: MemAccess) -> Vec<MicroOp> {
    if l {
        let mut ops = vec![MicroOp::MemRead(acc), MicroOp::Internal, MicroOp::Internal];
        if rd == 15 {
            ops.extend([MicroOp::Internal, MicroOp::Internal]);
        }
        ops
    } else {
        vec![MicroOp::MemWrite(acc), MicroOp::Internal]
    }
}

fn single_word(instr: u32, regs: &CpuRegisters, dec: &SingleDecoded) -> Option<Vec<MicroOp>> {
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
    // Same bus calls and order as legacy, so totals agree by construction.
    Some(single_ops(dec.l, dec.rd, acc))
}

fn single_half(instr: u32, regs: &CpuRegisters, dec: &SingleDecoded) -> Option<Vec<MicroOp>> {
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
    Some(single_ops(dec.l, dec.rd, acc))
}

/// Expand a Thumb instruction. `None` = not covered yet (legacy path).
/// `regs` snapshots stack/base pointers and STM store words at
/// queue-fill (pre-instruction state; registers are frozen across the
/// words of one instruction).
pub fn expand_thumb(instr: u16, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    // Conditional B (cond < 0xE); 0xDE00/0xDF00 fall through to the
    // UND/SWI traps below.
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
    // SWI: trap to the BIOS HLE (or the SVC vector when unhandled).
    if (instr & 0xFF00) == 0xDF00 {
        return Some(vec![MicroOp::TrapSwi((instr & 0xFF) as u8)]);
    }
    // UND: the 0xDE00 range traps to the undefined vector.
    if (instr & 0xFF00) == 0xDE00 {
        return Some(vec![MicroOp::TrapUnd]);
    }
    // UND: decoder gaps (0xB100-0xB3FF, 0xB600-0xBBFF, 0xBE00-0xBFFF)
    // fall into the legacy decoder's `_` arm (`handle_undefined`).
    if (0xB100..=0xB3FF).contains(&instr)
        || (0xB600..=0xBBFF).contains(&instr)
        || (0xBE00..=0xBFFF).contains(&instr)
    {
        return Some(vec![MicroOp::TrapUnd]);
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

/// Thumb LDMIA/STMIA, including the empty forms (single PC word with
/// +0x40 SP arithmetic on LDM, single R15+2 store on STM). Same block
/// slicing as PUSH/POP: per-word continuation at execution, batch
/// open/close and the single fetch-stream break instruction-scoped,
/// base writeback (LDM skips it when the base is loaded) in the end
/// commit.
fn expand_thumb_multiple(instr: u16, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    if instr >> 12 != 0xC {
        return None;
    }
    let load = (instr >> 11) & 1 == 1;
    let rb = ((instr >> 8) & 0x7) as usize;
    let rlist = instr & 0xFF;
    let base = regs.r(rb);
    if rlist == 0 {
        // Empty LDMIA/STMIA: the single PC word at [Rb] with Rb
        // advancing 0x40 (NOT the PUSH/POP shape: no batch framing,
        // writeback through `set_r`, store R15+2). Mirrors
        // `handle_empty_multiple` exactly (bases 5/2).
        let mut ops = vec![MicroOp::BlockEmpty(BlockEmptyEffect {
            load,
            addr: base,
            writeback_reg: Some((rb, base.wrapping_add(0x40))),
            store_value: regs.pc().wrapping_add(2),
            restore_cpsr: false,
            reset_sequential: false,
        })];
        ops.extend(vec![MicroOp::Internal; if load { 4 } else { 1 }]);
        return Some(ops);
    }
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
        first_addr: base,
    }));
    // Pad the legacy handler base (LDM 2+count, STM 1+count), except
    // single-register Thumb LDM (Break's ldmia r2!,{r3}): HW retires it
    // like a single LDR (1I, not 2I). Multi-word blocks (Timing OAM
    // 5-word cells pin 2I) keep 2.
    ops.extend(vec![
        MicroOp::Internal;
        if load {
            if count == 1 { 1 } else { 2 }
        } else {
            1
        }
    ]);
    Some(ops)
}

/// Thumb ALU remainder: move-shifted (0x0000..=0x17FF, 1 cycle),
/// ADD/SUB (0x1800..=0x1FFF, 1 cycle), register ALU (0x4000..=0x43FF
/// except MUL: 1 cycle, 2 for register shifts), hi-reg
/// (0x4400..=0x47FF except BX: 1 cycle, 3 for ADD/MOV to PC),
/// ADD SP/PC (0xA000..=0xAFFF) and SP offset (0xB000..=0xB0FF).
/// Expansion is [CommitThumb] padded to the legacy base; the commit
/// delegates, so only the cycle split is new.
pub(crate) fn expand_thumb_alu_rest(instr: u16) -> Option<Vec<MicroOp>> {
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

/// Thumb PUSH/POP, including the empty forms (single PC word with
/// +0x40 SP arithmetic on POP, single R15+2 store on PUSH). Mirrors
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
        return Some(expand_thumb_push_pop_empty(push, sp, regs));
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
        first_addr: if push { sp.wrapping_sub(count * 4) } else { sp },
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

/// Thumb empty PUSH/POP: PUSH stores R15+2 with SP moving one word
/// (base 2); POP loads PC with SP advancing 0x40 (base 6). Batched
/// like the legacy empty path; the op carries +1 with trailing
/// Internals to the base.
fn expand_thumb_push_pop_empty(push: bool, sp: u32, regs: &CpuRegisters) -> Vec<MicroOp> {
    let addr = if push { sp.wrapping_sub(4) } else { sp };
    let mut ops = vec![
        MicroOp::BlockStart(BlockStartEffect {
            is_load: !push,
            fetch_width: 2,
        }),
        MicroOp::BlockEmpty(BlockEmptyEffect {
            load: !push,
            addr,
            writeback_reg: None,
            store_value: regs.pc().wrapping_add(2),
            restore_cpsr: false,
            reset_sequential: true,
        }),
        MicroOp::BlockEnd(BlockEndEffect {
            sp: Some(if push {
                sp.wrapping_sub(4)
            } else {
                sp.wrapping_add(0x40)
            }),
            writeback: None,
            ldm_conflict: false,
            first_addr: addr,
        }),
    ];
    // Pad the legacy handler base: empty PUSH 2, empty POP 6
    // (the op already carries +1).
    ops.extend(vec![MicroOp::Internal; if push { 1 } else { 5 }]);
    ops
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
    bus.charge_fetch_stream_break(addr);
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
    bus.charge_fetch_stream_break(addr);
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
/// Thumb ADD/SUB (register and 3-bit immediate): native micro-op
/// implementation mirroring `thumb_opcodes::add_sub::handle` exactly.
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
/// mirroring `thumb_opcodes::alu::handle` exactly. MUL keeps the
/// 1S+mI timing with m from the incoming Rd value.
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
        1 + crate::cpu::arm_opcodes::multiply::multiplier_cycles(left)
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
/// mirroring `thumb_opcodes::alu::handle_load_address` exactly.
fn apply_load_address(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let sp = (instr >> 11) & 1 != 0;
    let rd = ((instr >> 8) & 0x7) as usize;
    let imm = ((instr & 0xFF) as u32) << 2;
    let base = if sp { regs.sp() } else { regs.pc() & !3 };
    regs.set_r(rd, base.wrapping_add(imm));
    1
}

/// Thumb ADD/SUB SP, #imm: native micro-op implementation mirroring
/// `thumb_opcodes::alu::handle_sp_offset` exactly.
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
/// routes 0x4700 to `MicroOp::Bx`): native micro-op implementation
/// mirroring `thumb_opcodes::hi_register::handle` exactly, including
/// the unreachable BX arm.
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
        refill_queue(regs, bus, pipeline, queue, is_thumb)?;
    }
    let pc = regs.pc();
    let op = queue.pop_front().expect("expansion never yields zero ops");
    let cycles = apply_op(regs, bus, op, pc, is_thumb);
    if queue.is_empty() {
        retire_step(regs, bus, pipeline, pc, is_thumb);
    }
    // No floor here: the driver floors once per instruction at retire,
    // exactly like the legacy step (per-op flooring would inflate
    // prefetch-erased instructions).
    Some(cycles + bus.take_access_wait_cycles())
}

/// SWP/SWPB (GBATEK ARM Single Data Swap): native micro-op
/// implementation mirroring `arm_opcodes::swp::handle` exactly
/// (atomic load-then-store with the HW bus lock: no DMA between the
/// pair, so expansion keeps it a single commit).
fn apply_swp(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let b = (instr >> 22) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let rm = (instr & 0xF) as usize;
    let addr = regs.r(rn);
    let rm_val = regs.r(rm);
    let mem_val = if b {
        bus.read8(addr) as u32
    } else {
        bus.read32(addr)
    };
    if b {
        bus.write8(addr, (rm_val & 0xFF) as u8);
    } else {
        bus.write32(addr, rm_val);
    }
    // Rd == R15 is UNPREDICTABLE (ARM ARM): skip the write like MUL.
    if rd != 15 {
        regs.set_r(rd, mem_val);
    }
    // Load+store breaks the fetch stream (once per instruction).
    bus.charge_fetch_stream_break(addr);
    4
}

/// MRS/MSR: native micro-op implementation mirroring
/// `arm_opcodes::psr_transfer::handle` exactly.
fn apply_psr(regs: &mut CpuRegisters, instr: u32) -> u32 {
    let psr = (instr >> 22) & 1 != 0; // 0=CPSR, 1=SPSR
    let is_mrs = (instr >> 21) & 1 == 0 && (instr & 0x0FBF0FFF) == 0x010F0000;
    if is_mrs {
        // MRS copies the selected status register into Rd.
        psr_read(regs, instr, psr);
    } else {
        psr_write(regs, instr, psr);
    }
    1
}

fn psr_read(regs: &mut CpuRegisters, instr: u32, saved: bool) {
    let rd = ((instr >> 12) & 0xF) as usize;
    // MRS with Rd=R15 is UNPREDICTABLE and must not branch: ignore the
    // write instead of latching PC to the PSR value via set_r(15).
    if rd == 15 {
        return;
    }
    let value = if saved { regs.spsr() } else { regs.cpsr() };
    regs.set_r(rd, value);
}

fn psr_write(regs: &mut CpuRegisters, instr: u32, saved: bool) {
    let operand = psr_operand(regs, instr);
    let mut field_mask = (instr >> 16) & 0xF;
    if regs.cpsr_mode() == 0x10 {
        field_mask &= 0x8;
    }
    let current = if saved { regs.spsr() } else { regs.cpsr() };
    let mut value = psr_apply_fields(current, operand, field_mask);
    if !saved {
        // GBATEK ARM PSR Transfer: "The T-bit may not be changed; for
        // THUMB/ARM switching use BX". Keep the live T bit on MSR to CPSR
        // (SPSR writes keep bit 5, which exception entry stores itself).
        value = (value & !(1 << 5)) | (current & (1 << 5));
        // Writing an illegal mode (< 0x10) keeps bit 4 set (HW-tested:
        // MSR 0x03 lands in 0x13, not 0x03).
        if value & 0x10 == 0 {
            value |= 0x10;
        }
    }
    if saved {
        regs.set_spsr(value);
    } else {
        regs.set_cpsr(value);
    }
}

fn psr_operand(regs: &CpuRegisters, instr: u32) -> u32 {
    if (instr >> 25) & 1 == 0 {
        return regs.r((instr & 0xF) as usize);
    }
    (instr & 0xFF).rotate_right(((instr >> 8) & 0xF) * 2)
}

fn psr_apply_fields(mut current: u32, operand: u32, mask: u32) -> u32 {
    // Field mask encoding: bit0=c, bit1=x, bit2=s, bit3=f.
    // ARM7TDMI defines control and NZCV fields here; reserved x/s bits stay unchanged.
    if mask & 1 != 0 {
        current = (current & 0xFFFFFF00) | (operand & 0xFF);
    }
    if mask & 8 != 0 {
        current = (current & 0x0FFFFFFF) | (operand & 0xF0000000);
    }
    current
}

/// ARM data-processing (register form, I==0): native micro-op
/// implementation mirroring `arm_opcodes::data_processing::handle`
/// exactly, including the register-shift +1I and R15-write refill.
fn apply_dp_reg(regs: &mut CpuRegisters, instr: u32) -> u32 {
    let i = (instr >> 25) & 1 != 0;
    let opcode = ((instr >> 21) & 0xF) as u8;
    let s = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let register_shift = !i && (instr >> 4) & 1 != 0;
    if register_shift {
        // Register-specified shifts take an internal cycle (GBATEK);
        // carried in the base below, no fetch-stream break.
    }
    // Operand2 always passes through the barrel shifter, including immediates.
    let (op2, shifter_carry) = dp_operand2(regs, instr, i);
    let rn_value = dp_register_operand(regs, rn, register_shift);
    let (result, carry, overflow) = dp_execute(opcode, rn_value, op2, shifter_carry, regs.cpsr_c());
    // TST/TEQ/CMP/CMN update flags without writing Rd.
    let flag_only = matches!(opcode, 0x8..=0xB);
    // USR/SYS have no SPSR (ARM ARM): exception-return restores only
    // restores only apply in modes with an SPSR bank.
    let has_spsr = !matches!(regs.cpsr_mode(), 0x10 | 0x1F);
    if flag_only && rd == 15 && s {
        // Unpredictable on later ARM cores; ARM7TDMI restores CPSR without writing PC.
        if has_spsr {
            regs.set_cpsr(regs.spsr());
            // Exception return: pipeline refill (+1S+1N).
            return 3 + u32::from(register_shift);
        }
        dp_update_flags(regs, opcode, result, carry, overflow);
        return 1 + u32::from(register_shift);
    }
    if !flag_only {
        dp_write_result(regs, rd, result, s);
    }
    if s && (!(rd == 15 && !flag_only) || !has_spsr) {
        dp_update_flags(regs, opcode, result, carry, overflow);
    }
    // GBATEK: data processing = 1S (+1I if SHIFT(Rs), +1S+1N if R15
    // written — pipeline refill, same convention as B).
    let mut cycles = 1 + u32::from(register_shift);
    if rd == 15 && !flag_only {
        cycles += 2;
    }
    cycles
}

fn dp_operand2(regs: &CpuRegisters, instr: u32, immediate: bool) -> (u32, bool) {
    if immediate {
        let imm = instr & 0xFF;
        let rot = ((instr >> 8) & 0xF) * 2;
        return dp_immediate_operand(imm, rot, regs.cpsr_c());
    }
    let rm = (instr & 0xF) as usize;
    let shift_type = ((instr >> 5) & 0b11) as u8;
    if (instr >> 4) & 1 != 0 {
        let value = dp_register_operand(regs, rm, true);
        let amount = regs.r(((instr >> 8) & 0xF) as usize) & 0xFF;
        barrel_shift_register(value, shift_type, amount, regs.cpsr_c())
    } else {
        let value = regs.r(rm);
        barrel_shift(value, shift_type, (instr >> 7) & 0x1F, regs.cpsr_c())
    }
}

fn dp_register_operand(regs: &CpuRegisters, register: usize, register_shift: bool) -> u32 {
    regs.r(register)
        .wrapping_add(u32::from(register == 15 && register_shift) * 4)
}

fn dp_immediate_operand(value: u32, rotation: u32, carry: bool) -> (u32, bool) {
    if rotation == 0 {
        (value, carry)
    } else {
        (
            value.rotate_right(rotation),
            value & (1 << (rotation - 1)) != 0,
        )
    }
}

fn dp_execute(
    opcode: u8,
    left: u32,
    right: u32,
    shift_carry: bool,
    carry: bool,
) -> (u32, bool, bool) {
    // Opcode map: AND, EOR, SUB, RSB, ADD, ADC, SBC, RSC,
    // TST, TEQ, CMP, CMN, ORR, MOV, BIC, MVN.
    match opcode {
        0x0 => (left & right, shift_carry, false),   // AND
        0x1 => (left ^ right, shift_carry, false),   // EOR
        0x2 | 0xA => dp_sub_with_flags(left, right), // SUB/CMP
        0x3 => dp_sub_with_flags(right, left),       // RSB
        0x4 | 0xB => dp_add_with_flags(left, right), // ADD/CMN
        0x5 => dp_adc_with_flags(left, right, u32::from(carry)), // ADC
        0x6 => dp_sbc_with_flags(left, right, u32::from(carry)), // SBC
        0x7 => dp_sbc_with_flags(right, left, u32::from(carry)), // RSC
        0x8 => (left & right, shift_carry, false),   // TST
        0x9 => (left ^ right, shift_carry, false),   // TEQ
        0xC => (left | right, shift_carry, false),   // ORR
        0xD => (right, shift_carry, false),          // MOV
        0xE => (left & !right, shift_carry, false),  // BIC
        0xF => (!right, shift_carry, false),         // MVN
        _ => unreachable!(),
    }
}

fn dp_write_result(regs: &mut CpuRegisters, destination: usize, result: u32, set_flags: bool) {
    regs.set_r(destination, result);
    if destination == 15 && set_flags {
        // Data-processing with S and Rd=PC returns from an exception via
        // SPSR — except in USR/SYS, which have none (ARM ARM): there
        // the PC write stands and flags update at the call site.
        if !matches!(regs.cpsr_mode(), 0x10 | 0x1F) {
            regs.set_cpsr(regs.spsr());
        }
    }
}

fn dp_update_flags(regs: &mut CpuRegisters, opcode: u8, result: u32, carry: bool, overflow: bool) {
    update_nz(regs, result);
    regs.set_cpsr_c(carry);
    // Logical operations preserve V; arithmetic operations replace it.
    if !matches!(opcode, 0x0 | 0x1 | 0x8 | 0x9 | 0xC..=0xF) {
        regs.set_cpsr_v(overflow);
    }
}

fn dp_add_with_flags(a: u32, b: u32) -> (u32, bool, bool) {
    let (r, c) = a.overflowing_add(b);
    let v = ((a ^ r) & (b ^ r) & 0x8000_0000) != 0;
    (r, c, v)
}

fn dp_sub_with_flags(a: u32, b: u32) -> (u32, bool, bool) {
    let (r, c) = a.overflowing_sub(b);
    let v = ((a ^ b) & (a ^ r) & 0x8000_0000) != 0;
    // Carry is NOT borrow
    (r, !c, v) // overflowing_sub returns borrow as carry; invert
}

fn dp_adc_with_flags(a: u32, b: u32, c_in: u32) -> (u32, bool, bool) {
    let (r1, c1) = a.overflowing_add(b);
    let (r, c2) = r1.overflowing_add(c_in);
    let c = c1 || c2;
    let signed = a as i32 as i64 + b as i32 as i64 + i64::from(c_in);
    let v = signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN);
    (r, c, v)
}

fn dp_sbc_with_flags(a: u32, b: u32, c_in: u32) -> (u32, bool, bool) {
    // SBC = A - B - !C
    let not_c = 1 - c_in;
    let (r1, c1) = a.overflowing_sub(b);
    let (r, c2) = r1.overflowing_sub(not_c);
    let c = !(c1 || c2);
    let signed = a as i32 as i64 - b as i32 as i64 - i64::from(not_c);
    let v = signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN);
    (r, c, v)
}

/// SWI trap: run the BIOS HLE dispatcher and carry its full charge
/// (SVC-vector entry on Unsupported), mirroring the legacy SWI
/// handlers (`arm_opcodes::swi`, `thumb_opcodes::branch`) exactly.
fn apply_trap_swi(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, swi: u8, is_thumb: bool) -> u32 {
    let back = if is_thumb { 2 } else { 4 };
    match crate::bios::handle_swi(regs, bus, swi) {
        crate::bios::SwiResult::Return(cycles) => {
            regs.set_pc(regs.pc().wrapping_sub(back));
            cycles
        }
        crate::bios::SwiResult::Branch(cycles) => cycles,
        crate::bios::SwiResult::Unsupported => {
            let return_address = regs.pc().wrapping_sub(back);
            regs.enter_exception(0x13, 0x08, return_address, true);
            3
        }
    }
}

/// Undefined-instruction trap: exception entry, mirroring the legacy
/// UND handlers exactly.
fn apply_trap_und(regs: &mut CpuRegisters, is_thumb: bool) -> u32 {
    let return_address = regs.pc().wrapping_sub(if is_thumb { 2 } else { 4 });
    regs.enter_exception(0x1B, 0x04, return_address, true);
    // GBATEK: Undefined = 2S+1I+1N = 4 in both states.
    4
}

/// Fill the queue from the executing instruction. `None` = uncovered
/// fill, queue untouched.
fn refill_queue(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    pipeline: &mut [u32; 2],
    queue: &mut VecDeque<MicroOp>,
    is_thumb: bool,
) -> Option<()> {
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
    Some(())
}

/// Execute one queued op; returns its cycle cost.
fn apply_op(
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

/// Retire: flush+refill on a PC write, else advance past the op.
fn retire_step(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    pipeline: &mut [u32; 2],
    pc: u32,
    is_thumb: bool,
) {
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
/// Semantics mirror `thumb_opcodes::move_shifted::handle` exactly
/// (bit-for-bit port of the shift/carry/flag behavior, including the
/// 1-cycle base return); the commit op carries the +1 cycle
/// separately, like the other arms here.
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

/// ARM MUL/MLA/UMULL/UMLAL/SMULL/SMLAL: native micro-op
/// implementation mirroring `arm_opcodes::multiply::handle` exactly.
/// The Booth-array carry model helpers stay in `multiply` (shared
/// leaf logic under its license notice) and are only called here.
fn apply_mul(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
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
            multiply_carry_lo(rm_value, rs_value, if accumulate { acc_lo } else { 0 })
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
}

fn apply_pcrel_read(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, r: PcRelRead, is_thumb: bool) {
    // Legacy-identical order: bus access, then fetch-stream-break.
    regs.set_r(r.rd, bus.read32(r.addr));
    bus.charge_fetch_stream_break(r.addr);
    // Thumb literal retire (loads the marker chain like a load).
    if is_thumb {
        bus.note_thumb_single_load(regs.pc(), r.addr);
    }
}

fn apply_block_word(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, w: BlockWord) {
    // Legacy-identical per-word order: continuation query, then
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

/// Empty-list block word: the single PC transfer, mirroring the
/// legacy empty path's exact bus-call sequence (access, then
/// fetch-stream break; batch framing comes from the surrounding
/// Start/End ops for Thumb, none for ARM).
fn apply_block_empty(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, e: BlockEmptyEffect) {
    if e.reset_sequential {
        bus.set_data_sequential(false);
    }
    if e.load {
        let target = bus.read_aligned32(e.addr);
        bus.charge_fetch_stream_break(e.addr);
        if let Some((reg, val)) = e.writeback_reg {
            regs.set_r(reg, val);
        }
        regs.set_pc(target);
        // LDM^ loading PC restores CPSR from SPSR. Unlike the
        // `BlockWord` path (which skips USR/SYS, following the
        // non-empty legacy handler), the empty legacy path restores
        // unconditionally — mirrored here.
        if e.restore_cpsr {
            regs.set_cpsr(regs.spsr());
        }
    } else {
        bus.write32(e.addr, e.store_value);
        bus.charge_fetch_stream_break(e.addr);
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
