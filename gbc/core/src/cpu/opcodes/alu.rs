//! ALU instructions: ADD/ADC/SUB/SBC/AND/XOR/OR/CP A,r8 and A,d8.

use crate::cpu::opcodes::CpuStepState;
use crate::cpu::{Lr35902Cpu, StepResult};
use crate::memory::GbcMemoryBus;

// ── ALU A, r8 (1-2 M-cycles) ───────────────────────────────
// reg: M1 (included in fetch): execute
// (HL): M1: read from (HL), M2: execute ALU + write result

pub(crate) struct AluAR8;
impl CpuStepState for AluAR8 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let op = core.opcode;
        let src = op & 0x07;
        let alu_op = (op >> 3) & 0x07;
        if src == 6 {
            // (HL) operand — 2 M-cycles
            match step {
                1 => {
                    core.operands[0] = bus.read(core.registers.hl());
                    StepResult::Continue
                }
                2 => {
                    alu_exec(core, alu_op, core.operands[0]);
                    StepResult::Exit
                }
                _ => unreachable!(),
            }
        } else {
            let v = match src {
                0 => core.registers.b,
                1 => core.registers.c,
                2 => core.registers.d,
                3 => core.registers.e,
                4 => core.registers.h,
                5 => core.registers.l,
                7 => core.registers.a,
                _ => 0,
            };
            alu_exec(core, alu_op, v);
            StepResult::Exit
        }
    }
}

// ── ALU A, d8 (2 M-cycles) ─────────────────────────────────

pub(crate) struct AluAD8<const OP: u8>;
impl<const OP: u8> CpuStepState for AluAD8<OP> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            2 => {
                alu_exec(core, OP, core.operands[0]);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

fn alu_exec(core: &mut Lr35902Cpu, op: u8, v: u8) {
    let a = core.registers.a;
    match op {
        0 => {
            let r = a.wrapping_add(v);
            core.registers.set_h((a & 0x0F) + (v & 0x0F) > 0x0F);
            core.registers.set_c((a as u16) + (v as u16) > 0xFF);
            core.registers.set_z(r == 0);
            core.registers.set_n(false);
            core.registers.a = r;
        }
        1 => {
            let c = core.registers.c_flag() as u8;
            let r = a.wrapping_add(v).wrapping_add(c);
            core.registers.set_h((a & 0x0F) + (v & 0x0F) + c > 0x0F);
            core.registers
                .set_c((a as u16) + (v as u16) + (c as u16) > 0xFF);
            core.registers.set_z(r == 0);
            core.registers.set_n(false);
            core.registers.a = r;
        }
        2 => {
            core.registers.set_h((a & 0x0F) < (v & 0x0F));
            core.registers.set_c(a < v);
            core.registers.a = a.wrapping_sub(v);
            core.registers.set_z(core.registers.a == 0);
            core.registers.set_n(true);
        }
        3 => {
            let c = core.registers.c_flag() as u8;
            let total = (v as u16) + (c as u16);
            core.registers.set_h((a & 0x0F) < (total as u8 & 0x0F));
            core.registers.set_c((a as u16) < total);
            core.registers.a = a.wrapping_sub(v).wrapping_sub(c);
            core.registers.set_z(core.registers.a == 0);
            core.registers.set_n(true);
        }
        4 => {
            core.registers.a &= v;
            core.registers.set_z(core.registers.a == 0);
            core.registers.set_n(false);
            core.registers.set_h(true);
            core.registers.set_c(false);
        }
        5 => {
            core.registers.a ^= v;
            core.registers.set_z(core.registers.a == 0);
            core.registers.set_n(false);
            core.registers.set_h(false);
            core.registers.set_c(false);
        }
        6 => {
            core.registers.a |= v;
            core.registers.set_z(core.registers.a == 0);
            core.registers.set_n(false);
            core.registers.set_h(false);
            core.registers.set_c(false);
        }
        7 => {
            core.registers.set_h((a & 0x0F) < (v & 0x0F));
            core.registers.set_c(a < v);
            core.registers.set_z(a.wrapping_sub(v) == 0);
            core.registers.set_n(true);
        }
        _ => {}
    }
}

// ── ADD HL, r16 (2 M-cycles) ───────────────────────────────

pub(crate) struct AddHlR16<const R: u8>;
impl<const R: u8> CpuStepState for AddHlR16<R> {
    fn exec(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                let hl = core.registers.hl();
                let v = match R {
                    0 => core.registers.bc(),
                    1 => core.registers.de(),
                    2 => core.registers.hl(),
                    _ => core.registers.sp,
                };
                core.registers.set_h((hl & 0x0FFF) + (v & 0x0FFF) > 0x0FFF);
                core.registers.set_c((hl as u32) + (v as u32) > 0xFFFF);
                core.registers.set_n(false);
                core.registers.set_hl(hl.wrapping_add(v));
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── ADD HL, SP (2 M-cycles) ────────────────────────────────

pub(crate) struct AddHlSp;
impl CpuStepState for AddHlSp {
    fn exec(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                let hl = core.registers.hl();
                let sp = core.registers.sp;
                core.registers.set_h((hl & 0x0FFF) + (sp & 0x0FFF) > 0x0FFF);
                core.registers.set_c((hl as u32) + (sp as u32) > 0xFFFF);
                core.registers.set_n(false);
                core.registers.set_hl(hl.wrapping_add(sp));
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── ADD SP, e (4 M-cycles) ─────────────────────────────────

pub(crate) struct AddSpE;
impl CpuStepState for AddSpE {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            2 | 3 => StepResult::Continue,
            4 => {
                let offset = core.operands[0] as i8;
                let sp = core.registers.sp;
                let r = sp.wrapping_add_signed(offset as i16);
                core.registers
                    .set_h((sp & 0x000F) + (offset as u8 as u16 & 0x000F) > 0x000F);
                core.registers
                    .set_c((sp & 0x00FF) + (offset as u8 as u16 & 0x00FF) > 0x00FF);
                core.registers.set_z(false);
                core.registers.set_n(false);
                core.registers.sp = r;
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}
