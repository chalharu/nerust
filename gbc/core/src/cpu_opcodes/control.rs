//! Control flow instructions: JP, JR, CALL, RET, RST.

use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::memory::GbcMemoryBus;

fn cond(c: u8, core: &Lr35902Cpu) -> bool {
    match c {
        0 => !core.registers.z_flag(),
        1 => core.registers.z_flag(),
        2 => !core.registers.c_flag(),
        _ => core.registers.c_flag(),
    }
}

// ── JP a16 (4 M-cycles) ────────────────────────────────────

pub(crate) struct JpA16;
impl CpuStepState for JpA16 {
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
                core.registers.pc = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

pub(crate) struct JpCond<const C: u8>;
impl<const C: u8> CpuStepState for JpCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let taken = cond(C, core);
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            2 => {
                core.operands[1] = core.pc_read(bus);
                StepResult::Continue
            }
            3 => {
                if !taken {
                    StepResult::Exit
                } else {
                    StepResult::Continue
                }
            }
            4 => {
                core.registers.pc = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

pub(crate) struct JpHl;
impl CpuStepState for JpHl {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        core.registers.pc = core.registers.hl();
        StepResult::Exit
    }
}

// ── JR e (3 M-cycles) ──────────────────────────────────────

pub(crate) struct Jr;
impl CpuStepState for Jr {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            2 => StepResult::Continue,
            3 => {
                core.registers.pc = core
                    .registers
                    .pc
                    .wrapping_add_signed(core.operands[0] as i8 as i16);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

pub(crate) struct JrCond<const C: u8>;
impl<const C: u8> CpuStepState for JrCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let taken = cond(C, core);
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            2 => {
                if !taken {
                    StepResult::Exit
                } else {
                    StepResult::Continue
                }
            }
            3 => {
                core.registers.pc = core
                    .registers
                    .pc
                    .wrapping_add_signed(core.operands[0] as i8 as i16);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── CALL (6 M-cycles) ──────────────────────────────────────

pub(crate) struct Call;
impl CpuStepState for Call {
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
                core.registers.sp = core.registers.sp.wrapping_sub(1);
                bus.write(core.registers.sp, (core.registers.pc >> 8) as u8);
                StepResult::Continue
            }
            6 => {
                core.registers.sp = core.registers.sp.wrapping_sub(1);
                bus.write(core.registers.sp, core.registers.pc as u8);
                core.registers.pc = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

pub(crate) struct CallCond<const C: u8>;
impl<const C: u8> CpuStepState for CallCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let taken = cond(C, core);
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            2 => {
                core.operands[1] = core.pc_read(bus);
                StepResult::Continue
            }
            3 => {
                if !taken {
                    StepResult::Exit
                } else {
                    StepResult::Continue
                }
            }
            4 => StepResult::Continue,
            5 => {
                core.registers.sp = core.registers.sp.wrapping_sub(1);
                bus.write(core.registers.sp, (core.registers.pc >> 8) as u8);
                StepResult::Continue
            }
            6 => {
                core.registers.sp = core.registers.sp.wrapping_sub(1);
                bus.write(core.registers.sp, core.registers.pc as u8);
                core.registers.pc = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── RET (4 M-cycles) ───────────────────────────────────────

pub(crate) struct Ret;
impl CpuStepState for Ret {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 | 2 => StepResult::Continue,
            3 => {
                core.operands[0] = bus.read(core.registers.sp);
                core.registers.sp = core.registers.sp.wrapping_add(1);
                StepResult::Continue
            }
            4 => {
                core.operands[1] = bus.read(core.registers.sp);
                core.registers.sp = core.registers.sp.wrapping_add(1);
                core.registers.pc = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

pub(crate) struct RetCond<const C: u8>;
impl<const C: u8> CpuStepState for RetCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let taken = cond(C, core);
        match step {
            1 => {
                if !taken {
                    StepResult::Exit
                } else {
                    StepResult::Continue
                }
            }
            2 => StepResult::Continue,
            3 => {
                core.operands[0] = bus.read(core.registers.sp);
                core.registers.sp = core.registers.sp.wrapping_add(1);
                StepResult::Continue
            }
            4 => StepResult::Continue,
            5 => {
                core.operands[1] = bus.read(core.registers.sp);
                core.registers.sp = core.registers.sp.wrapping_add(1);
                core.registers.pc = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

pub(crate) struct Reti;
impl CpuStepState for Reti {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 | 2 => StepResult::Continue,
            3 => {
                core.operands[0] = bus.read(core.registers.sp);
                core.registers.sp = core.registers.sp.wrapping_add(1);
                StepResult::Continue
            }
            4 => {
                core.operands[1] = bus.read(core.registers.sp);
                core.registers.sp = core.registers.sp.wrapping_add(1);
                core.registers.pc = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
                bus.set_ime(true);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

// ── RST (4 M-cycles) ───────────────────────────────────────

pub(crate) struct Rst<const V: u8>;
impl<const V: u8> CpuStepState for Rst<V> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 | 2 => StepResult::Continue,
            3 => {
                core.registers.sp = core.registers.sp.wrapping_sub(1);
                bus.write(core.registers.sp, (core.registers.pc >> 8) as u8);
                StepResult::Continue
            }
            4 => {
                core.registers.sp = core.registers.sp.wrapping_sub(1);
                bus.write(core.registers.sp, core.registers.pc as u8);
                core.registers.pc = V as u16 * 8;
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}
