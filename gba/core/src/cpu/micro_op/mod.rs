//! Unified micro-op CPU engine: decode expands each instruction into
//! [`MicroOp`]s, [`step_op`] drains them one per call, and the retire
//! step closes the instruction with instruction-atomic totals.
//!
//! Sub-modules form a DAG (nothing points back up):
//! `expand_arm`/`expand_thumb` share only the op types below,
//! `apply` adds the ARM commit helpers from `apply_arm`,
//! and the driver here calls down into all three.
mod apply;
mod apply_arm;
pub mod expand_arm;
pub mod expand_thumb;
pub(crate) mod ops_alu;
pub(crate) mod ops_block;
pub(crate) mod ops_block_words;
pub(crate) mod ops_branch;
pub(crate) mod ops_mem;

#[cfg(test)]
mod tests;

use crate::cpu_pipeline::fill_pipeline;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

use apply::apply_op;
use expand_arm::expand_arm_into;
use expand_thumb::expand_thumb_into;
use ops_alu::AluEffect;
use ops_block::{BlockEndEffect, BlockStartEffect};
use ops_block_words::{BlockEmptyEffect, BlockWord};
use ops_branch::{BranchEffect, MulEffect};
use ops_mem::{MemAccess, PcRelRead};

pub(crate) const HLE_IRQ_RETURN_TRAMPOLINE: u32 = 0x00000014;

/// HLE IRQ return slots, innermost last (mirrors the `GbaCpu` field).
pub(crate) type IrqReturnStack = Vec<(u32, [u32; 5])>;

/// Instruction decode buffer: stack-inline up to 8 ops (covers every
/// instruction but large block transfers, which spill once like `Vec`).
/// Replaces per-instruction heap allocation on the hot path; unlike
/// longer inline buffers, return-by-value copies stay small.
pub(crate) type MicroOpVec = smallvec::SmallVec<[MicroOp; 8]>;

/// One sub-instruction effect; effects land at execute-stage points with
/// the fixed bus-call order (access, then fetch-stream-break).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MicroOp {
    Internal,
    CommitAlu(AluEffect),
    /// ARM data-processing (register form): raw word, applied natively
    /// with its base returned separately (trailing `Internal`s pad it).
    /// Costs +1 here; no bus access, so only the cycle split is new.
    CommitDpReg(u32),
    /// Multiply commit: raw word plus mode. ARM short/long run the
    /// native apply; Thumb MUL carries the fetch break + P-ON erase
    /// plus the native ALU. Costs +1; the
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
    /// Unsupported).
    TrapSwi(u8),
    /// Undefined-instruction trap: exception entry
    /// (2S+1I+1N = 4 in both states).
    TrapUnd,
    TakenBranch(BranchEffect),
    MemRead(MemAccess),
    MemWrite(MemAccess),
    /// Thumb LDR-literal: pool address snapshotted at expansion
    /// (`(pc & !3) + imm`, pc frozen pre-instruction at execute stage).
    PcRelRead(PcRelRead),
    /// Open the block batch (`begin_block_batch(is_load, fetch_width)`).
    /// Zero-cost structural op.
    BlockStart(BlockStartEffect),
    BlockWord(BlockWord),
    /// Empty-list transfer (ARM LDM/STM with Rlist=0, Thumb PUSH/POP
    /// with no registers): the single PC word. Costs +1 like a block
    /// word; trailing `Internal`s pad the pinned base.
    BlockEmpty(BlockEmptyEffect),
    /// Close the batch, land end-commits, single fetch-stream break.
    /// Zero-cost structural op; trailing `Internal`s pad the base.
    BlockEnd(BlockEndEffect),
}

/// Returns the op's true cost with no floor (may be <= 0 from prefetch erases); driver floors once at retire.
/// `None` = uncovered fill, queue untouched.
pub fn step_op(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    pipeline: &mut [u32; 2],
    ops: &mut MicroOpVec,
    pos: &mut usize,
    is_thumb: bool,
    irq_return_stack: &mut IrqReturnStack,
) -> Option<i64> {
    if *pos >= ops.len() {
        refill_queue(regs, bus, pipeline, ops, pos, is_thumb)?;
    }
    let pc = regs.pc();
    // Index drain (no pop/shift): the op is `Copy`, so this is a plain
    // load; the consumed prefix is simply never revisited.
    let op = ops[*pos];
    *pos += 1;
    let cycles = apply_op(regs, bus, op, pc, is_thumb);
    let epilogue = if *pos >= ops.len() {
        retire_step(regs, bus, pipeline, pc, is_thumb, irq_return_stack)
    } else {
        0
    };
    // No floor here: the driver floors once per instruction at retire
    // (per-op flooring would inflate prefetch-erased instructions).
    Some(cycles + epilogue as i64 + bus.take_access_wait_cycles())
}

/// Fill the op buffer from the executing instruction. `None` = uncovered
/// fill, buffer untouched (still empty: refill only runs drained).
fn refill_queue(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    pipeline: &mut [u32; 2],
    ops: &mut MicroOpVec,
    pos: &mut usize,
    is_thumb: bool,
) -> Option<()> {
    // Speculative pure decode FIRST (touching bus/pipeline before
    // coverage is known would double-advance the pipeline on an
    // uncovered fill). Decodes straight into the reused inline buffer:
    // no SmallVec temp, no whole-buffer moves, no per-op queue pushes
    // (`clear` keeps the inline/spilled capacity across instructions).
    ops.clear();
    if is_thumb {
        expand_thumb_into((pipeline[0] & 0xFFFF) as u16, regs, ops)?;
    } else {
        expand_arm_into(pipeline[0], regs, ops)?;
    }
    *pos = 0;
    regs.clear_pc_written();
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
    Some(())
}

/// Retire: flush+refill on a PC write, else advance past the op.
fn retire_step(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    pipeline: &mut [u32; 2],
    pc: u32,
    is_thumb: bool,
    irq_return_stack: &mut IrqReturnStack,
) -> u32 {
    if regs.take_pc_written() {
        // A pc-write returning from a user IRQ handler through the HLE
        // trampoline runs the HLE-as-code epilogue (see
        // `bios_irq_epilogue_cycles`); other pc-writes just refill.
        let mut irq_epilogue = 0;
        if regs.pc() == HLE_IRQ_RETURN_TRAMPOLINE
            && let Some((return_address, saved)) = irq_return_stack.pop()
        {
            regs.set_cpsr(regs.spsr());
            for (register, value) in [0, 1, 2, 3, 12].into_iter().zip(saved) {
                regs.set_r(register, value);
            }
            // IRQ round-trip complete: the BIOS epilogue's last opcode
            // (0xE55EC002) is latched for protected reads (jsmolka t004).
            // After an IntrWait-family wake the BIOS exit path runs
            // instead (0xE3A02004, mgba-suite "BIOS load").
            if bus.take_bios_wait_exit() {
                bus.set_bios_prefetch(0xE3A02004);
            } else {
                bus.set_bios_prefetch(0xE55EC002);
            }
            regs.set_pc(return_address);
            // IntrWait re-halt: a non-awaited wake ISR returns here with
            // the awaited flags still clear; park again instead of
            // exiting the SWI early (see `rehalt_after_irq`).
            bus.rehalt_after_irq();
            irq_epilogue = bus.bios_irq_epilogue_cycles();
        }
        *pipeline = [0; 2];
        bus.set_current_pc(regs.pc());
        bus.invalidate_prefetch_for_branch();
        // NOTE: no `refill_prefetch_for_switch` here. Refilling on
        // every mode-switching pc-write was tried and FALSIFIED: the
        // unified engine matches HW without it (mgba-suite bx cells
        // pin the un-prefilled cost); refilling over-fills the buffer
        // and undercounts by 2-6 cycles there.
        fill_pipeline(regs, bus, pipeline);
        irq_epilogue
    } else {
        regs.set_pc(pc.wrapping_add(if is_thumb { 2 } else { 4 }));
        0
    }
}
