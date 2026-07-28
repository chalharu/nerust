use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::cpu_opcodes::helpers::reg;
use crate::cpu_opcodes::helpers::{read_r8, write_r8};
use crate::memory::GbcMemoryBus;

pub(crate) struct IncR8<const R: u8>;
impl<const R: u8> CpuStepState for IncR8<R> {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
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
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
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
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        // step == 1
        let v = match R {
            reg::BC => core.registers.bc(),
            reg::DE => core.registers.de(),
            reg::R16_HL => core.registers.hl(),
            _ => core.registers.sp,
        };
        let r = v.wrapping_add(1);
        match R {
            reg::BC => core.registers.set_bc(r),
            reg::DE => core.registers.set_de(r),
            reg::R16_HL => core.registers.set_hl(r),
            _ => core.registers.sp = r,
        }
        StepResult::Exit
    }
}

pub(crate) struct DecR16<const R: u8>;
impl<const R: u8> CpuStepState for DecR16<R> {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        // step == 1
        let v = match R {
            reg::BC => core.registers.bc(),
            reg::DE => core.registers.de(),
            reg::R16_HL => core.registers.hl(),
            _ => core.registers.sp,
        };
        let r = v.wrapping_sub(1);
        match R {
            reg::BC => core.registers.set_bc(r),
            reg::DE => core.registers.set_de(r),
            reg::R16_HL => core.registers.set_hl(r),
            _ => core.registers.sp = r,
        }
        StepResult::Exit
    }
}

pub(crate) struct IncSp;
impl CpuStepState for IncSp {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        // step == 1
        core.registers.sp = core.registers.sp.wrapping_add(1);
        StepResult::Exit
    }
}
pub(crate) struct DecSp;
impl CpuStepState for DecSp {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        // step == 1
        core.registers.sp = core.registers.sp.wrapping_sub(1);
        StepResult::Exit
    }
}

pub(crate) struct IncHlIndirect;
impl CpuStepState for IncHlIndirect {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            core.operands[0] = bus.read(core.registers.hl());
            return StepResult::Continue;
        }
        // step == 2
        let v = core.operands[0];
        let r = v.wrapping_add(1);
        bus.write(core.registers.hl(), r);
        core.registers.set_h((v & 0x0F) == 0x0F);
        core.registers.set_z(r == 0);
        core.registers.set_n(false);
        StepResult::Exit
    }
}

pub(crate) struct DecHlIndirect;
impl CpuStepState for DecHlIndirect {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            core.operands[0] = bus.read(core.registers.hl());
            return StepResult::Continue;
        }
        // step == 2
        let v = core.operands[0];
        let r = v.wrapping_sub(1);
        bus.write(core.registers.hl(), r);
        core.registers.set_h((v & 0x0F) == 0);
        core.registers.set_z(r == 0);
        core.registers.set_n(true);
        StepResult::Exit
    }
}
