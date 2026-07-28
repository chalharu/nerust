//! Load instructions: LD r8/r16, LDH, stack-relative.
//!
//! Each struct decomposes the instruction into M-cycle steps.

use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::memory::GbcMemoryBus;

// ── Helpers (shared in cpu_opcodes/helpers.rs) ───────────────
use crate::cpu_opcodes::helpers::{read_r8, read_r16, write_r8, write_r16};

fn r8(opcode: u8) -> u8 {
    opcode & 0x07
}
const R8_HL: u8 = 6;

/// Build a 16-bit address from operands[0..1].
fn addr16(core: &Lr35902Cpu) -> u16 {
    ((core.operands[1] as u16) << 8) | core.operands[0] as u16
}

// ── LD r16, d16 (3 M-cycles) ──────────────────────────────
// M1: opcode fetch
// M2: read lo byte from PC
// M3: read hi byte from PC, set r16

pub(crate) struct LdR16D16<const R: u8>;
impl<const R: u8> CpuStepState for LdR16D16<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            3 => {
                core.operands[1] = core.pc_read(bus);
                write_r16(core, R, addr16(core));
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── LD (r16mem), A (2 M-cycles) ────────────────────────────
// M1: opcode fetch (Fetch)
// M2: write A to (r16mem)

pub(crate) struct LdR16memA<const R: u8>;
impl<const R: u8> CpuStepState for LdR16memA<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                bus.write(read_r16(core, R), core.registers.a);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── LD A, (r16mem) (2 M-cycles) ────────────────────────────

pub(crate) struct LdAR16mem<const R: u8>;
impl<const R: u8> CpuStepState for LdAR16mem<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                core.registers.a = bus.read(read_r16(core, R));
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── LD r8, d8 (2 M-cycles) ────────────────────────────────
// M1: opcode fetch (Fetch)
// M2: read d8 from PC, write to r8

pub(crate) struct LdR8D8<const R: u8>;
impl<const R: u8> CpuStepState for LdR8D8<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                let v = core.pc_read(bus);
                write_r8(core, R, v);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── LD r8, r8 (1-2 M-cycles) ──────────────────────────────
// Reg-to-reg: 1 M-cycle (M1=opcode+execute in same cycle via Fetch)
// (HL) src: 2 M-cycles (M1=read (HL), M2=write to r8)
// (HL) dst: 2 M-cycles (M1=read r8, M2=write to (HL))... actually this is 2 cycles but we need to handle it

pub(crate) struct LdR8R8;
impl CpuStepState for LdR8R8 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let op = core.opcode;
        let src = r8(op);
        let dst = (op >> 3) & 0x07;

        if src == R8_HL {
            // LD r8, (HL) — 2 M-cycles
            match step {
                1 => StepResult::Continue,
                2 => {
                    let v = bus.read(core.registers.hl());
                    write_r8(core, dst, v);
                    StepResult::Exit
                }
                _ => unreachable!(),
            }
        } else if dst == R8_HL {
            // LD (HL), r8 — 2 M-cycles
            match step {
                1 => StepResult::Continue,
                2 => {
                    let v = read_r8(core, src);
                    bus.write(core.registers.hl(), v);
                    StepResult::Exit
                }
                _ => unreachable!(),
            }
        } else {
            // LD r8, r8 — 1 M-cycle
            write_r8(core, dst, read_r8(core, src));
            StepResult::Exit
        }
    }
}

// ── LD (HL+), A / LD A, (HL+) ──────────────────────────────

pub(crate) struct LdHliA;
impl CpuStepState for LdHliA {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                let a = core.registers.hl();
                bus.write(a, core.registers.a);
                core.registers.set_hl(a.wrapping_add(1));
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}
pub(crate) struct LdAHli;
impl CpuStepState for LdAHli {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                let a = core.registers.hl();
                core.registers.a = bus.read(a);
                core.registers.set_hl(a.wrapping_add(1));
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── LD (HL-), A / LD A, (HL-) ──────────────────────────────

pub(crate) struct LdHldA;
impl CpuStepState for LdHldA {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                let a = core.registers.hl();
                bus.write(a, core.registers.a);
                core.registers.set_hl(a.wrapping_sub(1));
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}
pub(crate) struct LdAHld;
impl CpuStepState for LdAHld {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                let a = core.registers.hl();
                core.registers.a = bus.read(a);
                core.registers.set_hl(a.wrapping_sub(1));
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── LD (HL), d8 (3 M-cycles) ───────────────────────────────

pub(crate) struct LdHlD8;
impl CpuStepState for LdHlD8 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            3 => {
                bus.write(core.registers.hl(), core.operands[0]);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── LD (a16), SP (5 M-cycles) ──────────────────────────────
// M1: fetch
// M2: read lo
// M3: read hi
// M4: internal
// M5: write SP to (a16)

pub(crate) struct LdA16Sp;
impl CpuStepState for LdA16Sp {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            3 => {
                core.operands[1] = core.pc_read(bus);
                StepResult::Continue
            }
            4 => {
                let addr = addr16(core);
                bus.write(addr, core.registers.sp as u8);
                core.operands[0] = addr as u8;
                core.operands[1] = (addr >> 8) as u8;
                StepResult::Continue
            }
            5 => {
                let addr = (core.operands[1] as u16) << 8 | core.operands[0] as u16;
                bus.write(addr.wrapping_add(1), (core.registers.sp >> 8) as u8);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── LD (a16), A (4 M-cycles) ───────────────────────────────

pub(crate) struct LdA16A;
impl CpuStepState for LdA16A {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            3 => {
                core.operands[1] = core.pc_read(bus);
                StepResult::Continue
            }
            4 => {
                bus.write(addr16(core), core.registers.a);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── LD A, (a16) (4 M-cycles) ───────────────────────────────

pub(crate) struct LdAA16;
impl CpuStepState for LdAA16 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            3 => {
                core.operands[1] = core.pc_read(bus);
                StepResult::Continue
            }
            4 => {
                core.registers.a = bus.read(addr16(core));
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── LD HL, SP+e (3 M-cycles) ───────────────────────────────
// M1: fetch (Fetch)
// M2: read e from PC
// M3: compute HL = SP + e

pub(crate) struct LdHlSpE;
impl CpuStepState for LdHlSpE {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            3 => {
                let offset = core.operands[0] as i8;
                let sp = core.registers.sp;
                let r = sp.wrapping_add_signed(offset as i16);
                core.registers
                    .set_h((sp & 0x000F) + (offset as u8 as u16 & 0x000F) > 0x000F);
                core.registers
                    .set_c((sp & 0x00FF) + (offset as u8 as u16 & 0x00FF) > 0x00FF);
                core.registers.set_z(false);
                core.registers.set_n(false);
                core.registers.set_hl(r);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}
