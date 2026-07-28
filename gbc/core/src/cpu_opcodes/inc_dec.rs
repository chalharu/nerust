//! INC/DEC instructions. Register indices follow Pan Docs: B=0,C=1,D=2,E=3,H=4,L=5,(HL)=6,A=7.

use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::cpu_opcodes::helpers::{read_r8, write_r8};
use crate::memory::GbcMemoryBus;

pub(crate) struct IncR8<const R: u8>;
impl<const R: u8> CpuStepState for IncR8<R> {
    fn exec(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let v = read_r8(core, R);
        let r = v.wrapping_add(1);
        write_r8(core, R, r);
        core.registers.set_h((v & 0x0F) == 0x0F);
        core.registers.set_z(r == 0);
        core.registers.set_n(false);
        StepResult::Exit
    }
}

pub(crate) struct DecR8<const R: u8>;
impl<const R: u8> CpuStepState for DecR8<R> {
    fn exec(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let v = read_r8(core, R);
        let r = v.wrapping_sub(1);
        write_r8(core, R, r);
        core.registers.set_h((v & 0x0F) == 0);
        core.registers.set_z(r == 0);
        core.registers.set_n(true);
        StepResult::Exit
    }
}

pub(crate) struct IncR16<const R: u8>;
impl<const R: u8> CpuStepState for IncR16<R> {
    fn exec(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let v = match R {
            0 => core.registers.bc(),
            1 => core.registers.de(),
            2 => core.registers.hl(),
            _ => core.registers.sp,
        };
        match R {
            0 => core.registers.set_bc(v.wrapping_add(1)),
            1 => core.registers.set_de(v.wrapping_add(1)),
            2 => core.registers.set_hl(v.wrapping_add(1)),
            _ => core.registers.sp = v.wrapping_add(1),
        }
        StepResult::Exit
    }
}
pub(crate) struct DecR16<const R: u8>;
impl<const R: u8> CpuStepState for DecR16<R> {
    fn exec(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let v = match R {
            0 => core.registers.bc(),
            1 => core.registers.de(),
            2 => core.registers.hl(),
            _ => core.registers.sp,
        };
        match R {
            0 => core.registers.set_bc(v.wrapping_sub(1)),
            1 => core.registers.set_de(v.wrapping_sub(1)),
            2 => core.registers.set_hl(v.wrapping_sub(1)),
            _ => core.registers.sp = v.wrapping_sub(1),
        }
        StepResult::Exit
    }
}

pub(crate) struct IncSp;
impl CpuStepState for IncSp {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        core.registers.sp = core.registers.sp.wrapping_add(1);
        StepResult::Exit
    }
}
pub(crate) struct DecSp;
impl CpuStepState for DecSp {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _: u8) -> StepResult {
        core.registers.sp = core.registers.sp.wrapping_sub(1);
        StepResult::Exit
    }
}

pub(crate) struct IncHlIndirect;
impl CpuStepState for IncHlIndirect {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let addr = core.registers.hl();
        match step {
            1 => {
                core.operands[0] = bus.read(addr);
                StepResult::Continue
            }
            2 => StepResult::Continue,
            3 => {
                let v = core.operands[0];
                let r = v.wrapping_add(1);
                bus.write(addr, r);
                core.registers.set_h((v & 0x0F) == 0x0F);
                core.registers.set_z(r == 0);
                core.registers.set_n(false);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

pub(crate) struct DecHlIndirect;
impl CpuStepState for DecHlIndirect {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let addr = core.registers.hl();
        match step {
            1 => {
                core.operands[0] = bus.read(addr);
                StepResult::Continue
            }
            2 => StepResult::Continue,
            3 => {
                let v = core.operands[0];
                let r = v.wrapping_sub(1);
                bus.write(addr, r);
                core.registers.set_h((v & 0x0F) == 0);
                core.registers.set_z(r == 0);
                core.registers.set_n(true);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}
