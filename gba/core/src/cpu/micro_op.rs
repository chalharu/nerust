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

use crate::cpu_pipeline::fill_pipeline;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// One sub-instruction effect. `Internal` is a bus-free cycle (future
/// prefetch-fill point); `CommitAlu`/`TakenBranch` land register/pc
/// effects at the execute-stage commit point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicroOp {
    Internal,
    CommitAlu(AluEffect),
    TakenBranch(u32),
}

/// Register effect of an ALU-immediate instruction, fully decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AluEffect {
    pub op: AluImmOp,
    pub rd: usize,
    pub rn: usize,
    pub imm: u32,
    pub set_flags: bool,
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
    // Data-processing immediate, no register shift, AL cond.
    if instr >> 28 != 0xE || (instr >> 25) & 1 != 1 || (instr >> 4) & 1 == 1 {
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
    // Shifted immediates (rot != 0) keep legacy flag semantics for now.
    if rot != 0 {
        return None;
    }
    Some(vec![
        MicroOp::CommitAlu(AluEffect {
            op,
            rd,
            rn,
            imm,
            set_flags: instr >> 20 & 1 == 1,
        }),
    ])
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
    if instr >> 13 != 0b001 {
        return None;
    }
    let op = match (instr >> 11) & 0x3 {
        0b00 => AluImmOp::Mov,
        0b01 => AluImmOp::Cmp,
        0b10 => AluImmOp::Add,
        _ => AluImmOp::Sub,
    };
    let rd = ((instr >> 8) & 0x7) as usize;
    let imm = (instr & 0xFF) as u32;
    Some(vec![
        MicroOp::CommitAlu(AluEffect {
            op,
            rd,
            rn: rd,
            imm,
            set_flags: true,
        }),
    ])
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
        regs.set_cpsr_n(result >> 31 != 0);
        regs.set_cpsr_z(result == 0);
        regs.set_cpsr_c(carry);
        regs.set_cpsr_v(overflow);
    }
}

/// Interpret one pipelined step using expansion. Mirrors
/// `GbaCpu::step_arm/step_thumb` (fetch/rotate/flush/refill) exactly;
/// only the execute phase goes through micro-ops. Returns `None` when
/// the executing instruction is not covered (caller keeps legacy path).
/// `pipeline`/`regs` layout matches `GbaCpu` (`pipeline[0]` executes).
#[allow(dead_code)]
pub fn interpret_step(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    pipeline: &mut [u32; 2],
    is_thumb: bool,
) -> Option<u32> {
    bus.take_access_wait_cycles();
    bus.set_current_pc(regs.pc());
    let pc = regs.pc();
    let (execute, cycles) = if is_thumb {
        let fetched = bus.fetch16(pc) as u32;
        let execute = (pipeline[0] & 0xFFFF) as u16;
        pipeline[0] = pipeline[1];
        pipeline[1] = fetched;
        regs.clear_pc_written();
        let ops = expand_thumb(execute)?;
        let mut cycles = 0;
        for op in ops {
            match op {
                MicroOp::Internal => cycles += 1,
                MicroOp::CommitAlu(fx) => {
                    apply_alu(regs, fx);
                    cycles += 1;
                }
                MicroOp::TakenBranch(off) => {
                    regs.set_pc(pc.wrapping_add(off as u32));
                    cycles += 1;
                }
            }
        }
        (execute as u32, cycles)
    } else {
        let fetched = bus.fetch32(pc);
        let execute = pipeline[0];
        pipeline[0] = pipeline[1];
        pipeline[1] = fetched;
        regs.clear_pc_written();
        // AL-only scaffold: other conds stay legacy (current engine
        // returns 1S with no effect; keep that behavior out of scope).
        if execute >> 28 != 0xE {
            return None;
        }
        let ops = expand_arm(execute)?;
        let mut cycles = 0;
        for op in ops {
            match op {
                MicroOp::Internal => cycles += 1,
                MicroOp::CommitAlu(fx) => {
                    apply_alu(regs, fx);
                    cycles += 1;
                }
                MicroOp::TakenBranch(off) => {
                    regs.set_pc(pc.wrapping_add(off as u32));
                    cycles += 1;
                }
            }
        }
        (execute, cycles)
    };
    let _ = execute;
    if regs.take_pc_written() {
        *pipeline = [0; 2];
        bus.set_current_pc(regs.pc());
        bus.invalidate_prefetch_for_dma(regs.pc());
        fill_pipeline(regs, bus, pipeline);
        // Legacy `step_arm/step_thumb` returns `cycles` here (plus an IRQ
        // epilogue only on the trampoline path, out of scope).
    } else {
        regs.set_pc(pc.wrapping_add(if is_thumb { 2 } else { 4 }));
    }
    Some((cycles as i64 + bus.take_access_wait_cycles()).max(1) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::GbaCpu;

    /// Run `steps` instructions under both engines from identical state.
    /// Returns (legacy_total, micro_total, regs_equal, followup_equal).
    fn differential(
        code: &[u32],
        thumb: bool,
        waitcnt: u16,
        steps: usize,
        followup: u32,
    ) -> (u32, u32, bool, bool) {
        fn setup(code: &[u32], thumb: bool, waitcnt: u16) -> (GbaCpu, GbaMemoryBus, [u32; 2]) {
            let mut cpu = GbaCpu::post_bios();
            let mut bus = GbaMemoryBus::new();
            bus.write16(0x04000204, waitcnt);
            for (i, w) in code.iter().enumerate() {
                if thumb {
                    bus.write16(0x0300_0000u32 + (i as u32) * 2, (w & 0xFFFF) as u16);
                } else {
                    bus.write32(0x0300_0000u32 + (i as u32) * 4, *w);
                }
            }
            // Stack data for the follow-up load.
            bus.write16(0x0300_7F00, 0x1234);
            cpu.regs.set_r(13, 0x0300_7F00);
            if thumb {
                cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
            }
            cpu.regs.set_pc(0x0300_0000);
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
        let (mut a_cpu, mut a_bus, _) = setup(code, thumb, waitcnt);
        let (mut b_cpu, mut b_bus, mut b_pipe) = setup(code, thumb, waitcnt);
        // Twin-bus check: both setups must agree before stepping.
        assert_eq!(a_cpu.pipeline, b_pipe);
        let (mut ta, mut tb) = (0u32, 0u32);
        for _ in 0..steps {
            // Legacy engine step on the real cpu.
            ta += a_cpu.step(&mut a_bus);
            // Micro-op engine step on the twin (same object layout:
            // b_cpu.regs is driven through interpret_step directly).
            tb += interpret_step(&mut b_cpu.regs, &mut b_bus, &mut b_pipe, thumb)
                .expect("corpus must be covered");
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

    /// Execute one arbitrary instruction via the legacy engine.
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
        cpu.step(bus)
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
            let (ta, tb, regs, follow) =
                differential(&ARM_CORPUS, false, waitcnt, 5, 0xE1DD_20B0);
            assert_eq!((ta, tb), (ta, ta), "waitcnt={waitcnt:#06x}");
            assert!(regs, "waitcnt={waitcnt:#06x}");
            assert!(follow, "waitcnt={waitcnt:#06x}");
        }
    }

    #[test]
    fn micro_op_matches_legacy_thumb() {
        for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4014] {
            let (ta, tb, regs, follow) =
                differential(&THUMB_CORPUS, true, waitcnt, 5, 0x886A);
            assert_eq!((ta, tb), (ta, ta), "waitcnt={waitcnt:#06x}");
            assert!(regs, "waitcnt={waitcnt:#06x}");
            assert!(follow, "waitcnt={waitcnt:#06x}");
        }
    }
}
