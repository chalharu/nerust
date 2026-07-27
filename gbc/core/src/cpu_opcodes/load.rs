//! Load instructions: LD r8/r16, LDH, stack-relative.
//!
//! Each struct decomposes the instruction into M-cycle steps.

use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::memory::GbcMemoryBus;

// ── Helpers ───────────────────────────────────────────────

const R8_B: u8 = 0;
const R8_C: u8 = 1;
const R8_D: u8 = 2;
const R8_E: u8 = 3;
const R8_H: u8 = 4;
const R8_L: u8 = 5;
const R8_HL: u8 = 6;
const R8_A: u8 = 7;

fn r8(opcode: u8) -> u8 {
    opcode & 0x07
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
fn r16_val(core: &Lr35902Cpu, idx: u8) -> u16 {
    match idx {
        0 => core.registers.bc(),
        1 => core.registers.de(),
        2 => core.registers.hl(),
        3 => core.registers.sp,
        _ => 0,
    }
}
fn set_r16(core: &mut Lr35902Cpu, idx: u8, v: u16) {
    match idx {
        0 => core.registers.set_bc(v),
        1 => core.registers.set_de(v),
        2 => core.registers.set_hl(v),
        3 => core.registers.sp = v,
        _ => {}
    }
}

// ── LD r16, d16 (3 M-cycles) ──────────────────────────────
// M1: opcode fetch (in Fetch state)
// M2: read lo byte from PC
// M3: read hi byte from PC, set r16

pub(crate) struct LdR16D16<const R: u8>;
impl<const R: u8> CpuStepState for LdR16D16<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
                core.operand_count = 1;
                StepResult::Continue
            }
            2 => {
                core.operands[1] = core.pc_read(bus);
                core.operand_count = 2;
                StepResult::Continue
            }
            3 => {
                let v = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
                set_r16(core, R, v);
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
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        bus.write(r16_val(core, R), core.registers.a);
        StepResult::Exit
    }
}

// ── LD A, (r16mem) (2 M-cycles) ────────────────────────────

pub(crate) struct LdAR16mem<const R: u8>;
impl<const R: u8> CpuStepState for LdAR16mem<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        core.registers.a = bus.read(r16_val(core, R));
        StepResult::Exit
    }
}

// ── LD r8, d8 (2 M-cycles) ────────────────────────────────
// M1: opcode fetch (Fetch)
// M2: read d8 from PC, write to r8

pub(crate) struct LdR8D8<const R: u8>;
impl<const R: u8> CpuStepState for LdR8D8<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                let v = core.pc_read(bus);
                core.operands[0] = v;
                StepResult::Continue
            }
            2 => {
                write_r8(core, R, core.operands[0]);
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
            // LD (HL), r8 — 2 M-cycles
            match step {
                1 => {
                    core.operands[0] = read_r8(core, src);
                    StepResult::Continue
                }
                2 => {
                    bus.write(core.registers.hl(), core.operands[0]);
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

// ── LD (HL+), A (2 M-cycles) ───────────────────────────────

pub(crate) struct LdHliA;
impl CpuStepState for LdHliA {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let addr = core.registers.hl();
        bus.write(addr, core.registers.a);
        core.registers.set_hl(addr.wrapping_add(1));
        StepResult::Exit
    }
}

// ── LD A, (HL+) (2 M-cycles) ───────────────────────────────

pub(crate) struct LdAHli;
impl CpuStepState for LdAHli {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let addr = core.registers.hl();
        core.registers.a = bus.read(addr);
        core.registers.set_hl(addr.wrapping_add(1));
        StepResult::Exit
    }
}

// ── LD (HL-), A (2 M-cycles) ───────────────────────────────

pub(crate) struct LdHldA;
impl CpuStepState for LdHldA {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let addr = core.registers.hl();
        bus.write(addr, core.registers.a);
        core.registers.set_hl(addr.wrapping_sub(1));
        StepResult::Exit
    }
}

// ── LD A, (HL-) (2 M-cycles) ───────────────────────────────

pub(crate) struct LdAHld;
impl CpuStepState for LdAHld {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let addr = core.registers.hl();
        core.registers.a = bus.read(addr);
        core.registers.set_hl(addr.wrapping_sub(1));
        StepResult::Exit
    }
}

// ── LD (HL), d8 (3 M-cycles) ───────────────────────────────
// M1: opcode fetch (Fetch)
// M2: read d8 from PC
// M3: write d8 to (HL)

pub(crate) struct LdHlD8;
impl CpuStepState for LdHlD8 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
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
            1 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            2 => {
                core.operands[1] = core.pc_read(bus);
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
}

// ── LD (a16), A (4 M-cycles) ───────────────────────────────

pub(crate) struct LdA16A;
impl CpuStepState for LdA16A {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            2 => {
                core.operands[1] = core.pc_read(bus);
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
}

// ── LD A, (a16) (4 M-cycles) ───────────────────────────────

pub(crate) struct LdAA16;
impl CpuStepState for LdAA16 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            2 => {
                core.operands[1] = core.pc_read(bus);
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
}

// ── LD HL, SP+e (3 M-cycles) ───────────────────────────────
// M1: fetch (Fetch)
// M2: read e from PC
// M3: compute HL = SP + e

pub(crate) struct LdHlSpE;
impl CpuStepState for LdHlSpE {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            2 => StepResult::Continue,
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
