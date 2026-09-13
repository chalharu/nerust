//! Micro-op scaffold for the per-cycle CPU remodel.
//!
//! Design: `nerust-docs/reference/gba/gba-12-per-cycle-cpu-design.md` §4.1.
//! Instruction-atomic decode is kept (existing handlers own semantics);
//! each covered instruction expands to micro-ops interpreted one step at
//! a time. GBATEK-total equivalence with [`crate::cpu::GbaCpu::step`] is
//! proven per covered class by the differential tests at the bottom.
//!
//! Covered so far (slice 1): ALU-immediate without register shift
//! (ARM MOV/ADD/SUB/CMP, Thumb MOV/CMP/ADD/SUB) and unconditional
//! branches (ARM B, Thumb B). Everything else returns `None` from the
//! expanders and stays on the legacy path.

use std::collections::VecDeque;

use crate::cpu_pipeline::fill_pipeline;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// One sub-instruction effect. `Internal` is a bus-free cycle (future
/// prefetch-fill point); `CommitAlu`/`TakenBranch`/`MemRead`/`MemWrite`
/// land register/pc/memory effects at their execute-stage points, with
/// the legacy handler's exact bus-call order (access, then
/// fetch-stream-break).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicroOp {
    Internal,
    CommitAlu(AluEffect),
    TakenBranch(u32),
    MemRead(MemAccess),
    MemWrite(MemAccess),
}

/// One data access. `addr = base +/- offset` is resolved at interpret
/// time from live registers (matches handler evaluation order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemAccess {
    pub width: u8,
    pub rd: usize,
    pub rn: usize,
    pub offset: u32,
    pub subtract: bool,
    pub is_sp: bool,
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
}

/// Covered ALU-immediate operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AluImmOp {
    Mov,
    Add,
    Sub,
    Cmp,
}

/// Expand an ARM instruction. `None` = not covered yet (legacy path).
pub fn expand_arm(instr: u32) -> Option<Vec<MicroOp>> {
    // B (AL only for the scaffold).
    if instr >> 24 == 0xEA {
        let offset = ((instr & 0x00FF_FFFF) as i32) << 2;
        let offset = (offset << 6) >> 6;
        return Some(vec![MicroOp::Internal, MicroOp::Internal, MicroOp::TakenBranch(offset as u32)]);
    }
    if instr >> 28 != 0xE {
        return None;
    }
    if let Some(ops) = expand_arm_alu_imm(instr) {
        return Some(ops);
    }
    if let Some(ops) = expand_arm_single(instr) {
        return Some(ops);
    }
    None
}

/// ARM data-processing immediate, no R15, no S+rotate (see gate above).
fn expand_arm_alu_imm(instr: u32) -> Option<Vec<MicroOp>> {
    // Data-processing class (bits27-26 == 00): excludes SWI (11) and
    // anything outside DP. Immediate form, no register shift.
    if (instr >> 26) & 0x3 != 0 || (instr >> 25) & 1 != 1 || (instr >> 4) & 1 == 1 {
        return None;
    }
    let opcode = ((instr >> 21) & 0xF) as u8;
    let op = match opcode {
        0xD => AluImmOp::Mov,
        0x4 => AluImmOp::Add,
        0x2 => AluImmOp::Sub,
        0xA => AluImmOp::Cmp,
        _ => return None,
    };
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    if rn == 15 || rd == 15 {
        return None;
    }
    let rot = ((instr >> 8) & 0xF) * 2;
    let imm = (instr & 0xFF).rotate_right(rot);
    // Shifted immediates keep legacy flag semantics when S is set
    // (shifter carry-out); without S the rotation is flag-neutral.
    if rot != 0 && instr >> 20 & 1 == 1 {
        return None;
    }
    Some(vec![
        MicroOp::CommitAlu(AluEffect {
            op,
            rd,
            rn,
            imm,
            set_flags: instr >> 20 & 1 == 1,
            thumb_mov: false,
        }),
    ])
}

/// ARM LDR/STR word-immediate and LDRH/STRH unsigned-immediate
/// (P=1, W=0, no R15). Load expands to [Read, I, I] (= 3) and store to
/// [Write, I] (= 2), matching `single_transfer`/`halfword_transfer`.
fn expand_arm_single(instr: u32) -> Option<Vec<MicroOp>> {
    if (instr >> 24) & 1 != 1 || (instr >> 21) & 1 == 1 {
        return None;
    }
    let l = (instr >> 20) & 1 == 1;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    if rn == 15 || rd == 15 {
        return None;
    }
    let subtract = (instr >> 23) & 1 == 0;
    // Word-immediate class (bits27-26 == 01, I == 0, B == 0).
    if (instr >> 26) & 0x3 == 0b01 && (instr >> 25) & 1 == 0 && (instr >> 22) & 1 == 0 {
        let acc = MemAccess { width: 4, rd, rn, offset: instr & 0xFFF, subtract, is_sp: false };
        // Cycle sums match the handler returns (load 3, store 2):
        // the bus calls charge data/erase/break, Internals pad the
        // GBATEK 1S base + I. (Attribution internalizes under the
        // arbiter; totals are what the differential pins.)
        // Fixed bases match the handler returns (load 3, store 2):
        // the issue clock (+1) plus commit/internal ones. Totals equal
        // legacy by construction (same bus calls, same bases).
        return Some(if l {
            vec![
                MicroOp::MemRead(acc),
                MicroOp::Internal,
                MicroOp::Internal,
            ]
        } else {
            vec![MicroOp::MemWrite(acc), MicroOp::Internal]
        });
    }
    // Halfword-immediate class (bits27-25 == 000, imm bit22) with the
    // full 1SH1 tag (bits7-4 == 1011, S:H == 0:1 unsigned). The full tag
    // is load-bearing: data-processing register-shift BIC/MVN/RSB/... can
    // otherwise mimic the halfword shape (e.g. `bic rd, rn, rm, lsl #N`).
    if (instr >> 25) & 0x7 == 0 && (instr >> 22) & 1 == 1 && (instr >> 4) & 0xF == 0xB {
        let offset = (((instr >> 8) & 0xF) << 4) | (instr & 0xF);
        let acc = MemAccess { width: 2, rd, rn, offset, subtract, is_sp: false };
        // Cycle sums match the handler returns (load 3, store 2):
        // the bus calls charge data/erase/break, Internals pad the
        // GBATEK 1S base + I. (Attribution internalizes under the
        // arbiter; totals are what the differential pins.)
        // Fixed bases match the handler returns (load 3, store 2):
        // the issue clock (+1) plus commit/internal ones. Totals equal
        // legacy by construction (same bus calls, same bases).
        return Some(if l {
            vec![
                MicroOp::MemRead(acc),
                MicroOp::Internal,
                MicroOp::Internal,
            ]
        } else {
            vec![MicroOp::MemWrite(acc), MicroOp::Internal]
        });
    }
    None
}

/// Expand a Thumb instruction. `None` = not covered yet (legacy path).
pub fn expand_thumb(instr: u16) -> Option<Vec<MicroOp>> {
    // B (unconditional).
    if instr >> 11 == 0b11100 {
        let offset = ((instr & 0x7FF) as i32) << 1;
        let offset = (offset << 20) >> 20;
        return Some(vec![MicroOp::Internal, MicroOp::Internal, MicroOp::TakenBranch(offset as u32)]);
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
        return Some(vec![
            MicroOp::CommitAlu(AluEffect {
                op,
                rd,
                rn: rd,
                imm,
                set_flags: true,
                thumb_mov: matches!(op, AluImmOp::Mov),
            }),
        ]);
    }
    expand_thumb_load_store(instr)
}

/// Thumb word LDR/STR (immediate offset and SP-relative) and
/// LDRH/STRH immediate. Same [Read, I, I] / [Write, I] shape as ARM.
pub fn expand_thumb_load_store(instr: u16) -> Option<Vec<MicroOp>> {
    // SP-relative (1001): addr = SP + imm8<<2.
    if instr >> 12 == 0b1001 {
        let l = (instr >> 11) & 1 == 1;
        let rd = ((instr >> 8) & 0x7) as usize;
        let acc = MemAccess {
            width: 4,
            rd,
            rn: 13,
            offset: ((instr & 0xFF) as u32) << 2,
            subtract: false,
            is_sp: true,
        };
        // Cycle sums match the handler returns (load 3, store 2):
        // the bus calls charge data/erase/break, Internals pad the
        // GBATEK 1S base + I. (Attribution internalizes under the
        // arbiter; totals are what the differential pins.)
        // Fixed bases match the handler returns (load 3, store 2):
        // the issue clock (+1) plus commit/internal ones. Totals equal
        // legacy by construction (same bus calls, same bases).
        return Some(if l {
            vec![
                MicroOp::MemRead(acc),
                MicroOp::Internal,
                MicroOp::Internal,
            ]
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
            subtract: false,
            is_sp: false,
        };
        // Cycle sums match the handler returns (load 3, store 2):
        // the bus calls charge data/erase/break, Internals pad the
        // GBATEK 1S base + I. (Attribution internalizes under the
        // arbiter; totals are what the differential pins.)
        // Fixed bases match the handler returns (load 3, store 2):
        // the issue clock (+1) plus commit/internal ones. Totals equal
        // legacy by construction (same bus calls, same bases).
        return Some(if l {
            vec![
                MicroOp::MemRead(acc),
                MicroOp::Internal,
                MicroOp::Internal,
            ]
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
            subtract: false,
            is_sp: false,
        };
        // Cycle sums match the handler returns (load 3, store 2):
        // the bus calls charge data/erase/break, Internals pad the
        // GBATEK 1S base + I. (Attribution internalizes under the
        // arbiter; totals are what the differential pins.)
        // Fixed bases match the handler returns (load 3, store 2):
        // the issue clock (+1) plus commit/internal ones. Totals equal
        // legacy by construction (same bus calls, same bases).
        return Some(if l {
            vec![
                MicroOp::MemRead(acc),
                MicroOp::Internal,
                MicroOp::Internal,
            ]
        } else {
            vec![MicroOp::MemWrite(acc), MicroOp::Internal]
        });
    }
    None
}

fn resolve_addr(regs: &CpuRegisters, a: MemAccess) -> u32 {
    let base = if a.is_sp { regs.sp() } else { regs.r(a.rn) };
    if a.subtract {
        base.wrapping_sub(a.offset)
    } else {
        base.wrapping_add(a.offset)
    }
}

/// Apply one data access with the legacy handler's exact bus-call order
/// (access, writeback, then fetch-stream-break). The issue clock (+1)
/// lands at the call site; the bus calls charge into `access_wait_cycles`.
fn apply_read(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, a: MemAccess) {
    let addr = resolve_addr(regs, a);
    let v = match a.width {
        4 => bus.read32(addr),
        _ => bus.read_ldr_halfword(addr),
    };
    regs.set_r(a.rd, v);
    bus.charge_fetch_stream_break();
}

/// Apply one data store with the legacy handler's exact bus-call order
/// (access, then fetch-stream-break). The issue clock (+1) lands at the
/// call site; the bus calls charge into `access_wait_cycles`.
fn apply_write(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, a: MemAccess) {
    let addr = resolve_addr(regs, a);
    match a.width {
        4 => bus.write32(addr, regs.r(a.rd)),
        _ => bus.write16(addr, regs.r(a.rd) as u16),
    }
    bus.charge_fetch_stream_break();
}

fn apply_alu(regs: &mut CpuRegisters, fx: AluEffect) {
    let (result, carry, overflow) = match fx.op {
        AluImmOp::Mov => (fx.imm, regs.cpsr_c(), false),
        AluImmOp::Add => {
            let (r, c) = regs.r(fx.rn).overflowing_add(fx.imm);
            let v = ((!(regs.r(fx.rn) ^ fx.imm)) & (regs.r(fx.rn) ^ r) & 0x8000_0000) != 0;
            (r, c, v)
        }
        AluImmOp::Sub | AluImmOp::Cmp => {
            let (r, b) = regs.r(fx.rn).overflowing_sub(fx.imm);
            let v = ((regs.r(fx.rn) ^ fx.imm) & (regs.r(fx.rn) ^ r) & 0x8000_0000) != 0;
            (r, !b, v)
        }
    };
    if !matches!(fx.op, AluImmOp::Cmp) {
        regs.set_r(fx.rd, result);
    }
    if fx.set_flags {
        // N: Thumb MOV-imm forces 0 (legacy quirk); otherwise bit 31.
        // V: MOV preserves it in both modes (logical class); arithmetic
        // replaces it. C: shift-carry (preserved here: rot == 0 or S == 0
        // gate) for MOV, computed carry otherwise.
        regs.set_cpsr_n(if fx.thumb_mov { false } else { result >> 31 != 0 });
        regs.set_cpsr_z(result == 0);
        regs.set_cpsr_c(carry);
        if !matches!(fx.op, AluImmOp::Mov) {
            regs.set_cpsr_v(overflow);
        }
    }
}

/// Interpret one pipelined step using expansion. Mirrors
/// `GbaCpu::step_arm/step_thumb` (fetch/rotate/flush/refill) exactly;
/// only the execute phase goes through micro-ops. Returns `None` when
/// the executing instruction is not covered (caller keeps legacy path).
/// `pipeline`/`regs` layout matches `GbaCpu` (`pipeline[0]` executes).
#[allow(dead_code)]
/// Queue-driven single micro-op step (per-cycle remodel slice 3b).
/// Executes exactly one micro-op per call so the driver can tick
/// peripherals between ops. Returns the op's true cost WITHOUT any
/// floor (possibly zero or negative: prefetch erases overlap fills);
/// the driver floors once per instruction at retire, exactly like the
/// legacy step. `None` = uncovered fill with zero state change.
///
/// True-cost attribution (sums to the handler return per class):
/// ALU/Branch ops = 1; load issue = 1 + data waits, commit = 1,
/// trailing internal = 1 (fixed 3); store issue = 1 + waits,
/// trailing internal = 1 (fixed 2). Bus waits land via
/// `access_wait_cycles` takes, same calls and order as legacy.
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
            expand_thumb((pipeline[0] & 0xFFFF) as u16)?
        } else {
            // AL-only: other conds stay legacy.
            if pipeline[0] >> 28 != 0xE {
                return None;
            }
            expand_arm(pipeline[0])?
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
        MicroOp::TakenBranch(off) => {
            regs.set_pc(pc.wrapping_add(off));
            cycles += 1;
        }
        MicroOp::MemRead(a) => {
            // Legacy-identical issue: bus access, writeback and break in
            // the issue tick. (A deferred-commit timer re-sample was tried
            // here and FALSIFIED — it breaks 12 nba DMA pins that pin
            // issue-time sampling; see the design doc. The queue/drain
            // machinery stays as the verified-neutral execution model.)
            apply_read(regs, bus, a);
            cycles += 1;
        }
        MicroOp::MemWrite(a) => {
            apply_write(regs, bus, a);
            cycles += 1;
        }
    }
    if queue.is_empty() {
        if regs.take_pc_written() {
            *pipeline = [0; 2];
            bus.set_current_pc(regs.pc());
            bus.invalidate_prefetch_for_dma(regs.pc());
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
        let (mut a_cpu, mut a_bus, _) =
            setup(code, thumb, waitcnt, code_base, cart.clone(), mem_init, reg_init);
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
                acc += step_op(&mut b_cpu.regs, &mut b_bus, &mut b_pipe, &mut b_queue, thumb)
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
    const THUMB_LS_CORPUS: [u32; 7] = [
        0x6088, // str r0, [r1, #8]
        0x688B, // ldr r3, [r1, #8]
        0x8090, // strh r0, [r2, #4]
        0x8894, // ldrh r4, [r2, #4]
        0x9010, // str r0, [sp, #0x40]
        0x9D10, // ldr r5, [sp, #0x40]
        0x2707, // mov r7, #7
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
        const SUITE: &[u8] =
            include_bytes!("../../../../roms/gba/mgba-suite/suite.gba");
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

    /// System-cadence tick parity: run a straight-line snippet under the
    /// legacy step+countdown rhythm vs the micro acc-loop drain rhythm,
    /// ticking the bus once per elapsed tick in both, with TM0 running.
    /// Returns (legacy_ticks, micro_ticks, legacy_tm0, micro_tm0).
    /// TIMER POLLUTION NOTE: this is the load-bearing invariant behind
    /// the system.rs drain loop — any tick-count divergence here moves
    /// every timer-measured suite cell.
    fn tick_parity(
        code: &[u32],
        thumb: bool,
        waitcnt: u16,
        code_base: u32,
        reg_init: &[(usize, u32)],
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
        for _ in 0..code.len() {
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
        while retired < code.len() {
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
        let (at, bt, av, bv) = tick_parity(&code, true, 0x0000, 0x0300_0000, &[]);
        assert_eq!((at, bt), (at, at), "ticks diverge");
        assert_eq!((av, bv), (av, av), "timer span diverges");
    }
}
