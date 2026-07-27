//! Miscellaneous instructions: NOP, STOP, HALT, DAA, rotates, DI, EI, LDH.

use crate::cpu::opcodes::CpuStepState;
use crate::cpu::{Lr35902Cpu, StepResult};
use crate::memory::GbcMemoryBus;

pub(crate) struct Nop;
impl CpuStepState for Nop {
    fn exec(_: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        StepResult::Exit
    }
}

pub(crate) struct Invalid;
impl CpuStepState for Invalid {
    fn exec(_: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        StepResult::Exit
    }
}

/// Invalid opcode that reads operand bytes and consumes cycles.
/// B = total byte count (1-3), C = total M-cycle count (0-6)
pub(crate) struct InvalidOp<const B: u8, const M: u8>;
impl<const B: u8, const M: u8> CpuStepState for InvalidOp<B, M> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step < B {
            // Read and discard operand byte
            core.pc_read(bus);
        }
        if step < M {
            StepResult::Continue
        } else {
            StepResult::Exit
        }
    }
}

pub(crate) struct Halt;
impl CpuStepState for Halt {
    fn exec(_: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _: u8) -> StepResult {
        bus.halt_cpu();
        StepResult::Exit
    }
}

pub(crate) struct Stop;
impl CpuStepState for Stop {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _: u8) -> StepResult {
        core.registers.pc = core.registers.pc.wrapping_add(1);
        bus.stop();
        StepResult::Exit
    }
}

pub(crate) struct Di;
impl CpuStepState for Di {
    fn exec(_: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _: u8) -> StepResult {
        bus.set_ime(false);
        StepResult::Exit
    }
}

pub(crate) struct Ei;
impl CpuStepState for Ei {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        core.ime_delayed = true;
        StepResult::Exit
    }
}

// ── Rotates (1 M-cycle) ────────────────────────────────────

pub(crate) struct Rlca;
impl CpuStepState for Rlca {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        let c = core.registers.a & 0x80 != 0;
        core.registers.a = (core.registers.a << 1) | c as u8;
        core.registers.set_z(false);
        core.registers.set_n(false);
        core.registers.set_h(false);
        core.registers.set_c(c);
        StepResult::Exit
    }
}
pub(crate) struct Rrca;
impl CpuStepState for Rrca {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        let c = core.registers.a & 0x01 != 0;
        core.registers.a = (core.registers.a >> 1) | if c { 0x80 } else { 0 };
        core.registers.set_z(false);
        core.registers.set_n(false);
        core.registers.set_h(false);
        core.registers.set_c(c);
        StepResult::Exit
    }
}
pub(crate) struct Rla;
impl CpuStepState for Rla {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        let c = core.registers.a & 0x80 != 0;
        core.registers.a = (core.registers.a << 1) | core.registers.c_flag() as u8;
        core.registers.set_z(false);
        core.registers.set_n(false);
        core.registers.set_h(false);
        core.registers.set_c(c);
        StepResult::Exit
    }
}
pub(crate) struct Rra;
impl CpuStepState for Rra {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        let c = core.registers.a & 0x01 != 0;
        core.registers.a = (core.registers.a >> 1) | if core.registers.c_flag() { 0x80 } else { 0 };
        core.registers.set_z(false);
        core.registers.set_n(false);
        core.registers.set_h(false);
        core.registers.set_c(c);
        StepResult::Exit
    }
}

// ── DAA / CPL / SCF / CCF (1 M-cycle) ──────────────────────

pub(crate) struct Daa;
impl CpuStepState for Daa {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
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
        core.registers.set_z(a == 0);
        core.registers.set_h(false);
        core.registers.set_c(carry);
        StepResult::Exit
    }
}
pub(crate) struct Cpl;
impl CpuStepState for Cpl {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        core.registers.a = !core.registers.a;
        core.registers.set_n(true);
        core.registers.set_h(true);
        StepResult::Exit
    }
}
pub(crate) struct Scf;
impl CpuStepState for Scf {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        core.registers.set_n(false);
        core.registers.set_h(false);
        core.registers.set_c(true);
        StepResult::Exit
    }
}
pub(crate) struct Ccf;
impl CpuStepState for Ccf {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        let c = core.registers.c_flag();
        core.registers.set_n(false);
        core.registers.set_h(false);
        core.registers.set_c(!c);
        StepResult::Exit
    }
}

// ── LDH (3 M-cycles) ───────────────────────────────────────

pub(crate) struct LdhA8A;
impl CpuStepState for LdhA8A {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
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
}
pub(crate) struct LdhAA8;
impl CpuStepState for LdhAA8 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
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
}

// ── LD (C), A / LD A, (C) (2 M-cycles) ─────────────────────

pub(crate) struct LdCA;
impl CpuStepState for LdCA {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _: u8) -> StepResult {
        bus.write(0xFF00 | core.registers.c as u16, core.registers.a);
        StepResult::Exit
    }
}
pub(crate) struct LdAC;
impl CpuStepState for LdAC {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _: u8) -> StepResult {
        core.registers.a = bus.read(0xFF00 | core.registers.c as u16);
        StepResult::Exit
    }
}

// ── LD SP, HL (2 M-cycles) ─────────────────────────────────

pub(crate) struct LdSpHl;
impl CpuStepState for LdSpHl {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        core.registers.sp = core.registers.hl();
        StepResult::Exit
    }
}
