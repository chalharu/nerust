//! Stack instructions: PUSH, POP.

use crate::cpu_core::Lr35902Cpu;
use crate::cpu_opcodes::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::memory::GbcMemoryBus;

pub(crate) struct Push<const R: u8>;
impl<const R: u8> CpuStepState for Push<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let v = if R == 3 {
            core.registers.af()
        } else {
            match R {
                0 => core.registers.bc(),
                1 => core.registers.de(),
                2 => core.registers.hl(),
                _ => 0,
            }
        };
        match step {
            1 | 2 => StepResult::Continue,
            3 => {
                core.registers.sp = core.registers.sp.wrapping_sub(1);
                bus.write(core.registers.sp, (v >> 8) as u8);
                StepResult::Continue
            }
            4 => {
                core.registers.sp = core.registers.sp.wrapping_sub(1);
                bus.write(core.registers.sp, v as u8);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

pub(crate) struct Pop<const R: u8>;
impl<const R: u8> CpuStepState for Pop<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = bus.read(core.registers.sp);
                core.registers.sp = core.registers.sp.wrapping_add(1);
                StepResult::Continue
            }
            2 => {
                core.operands[1] = bus.read(core.registers.sp);
                core.registers.sp = core.registers.sp.wrapping_add(1);
                StepResult::Continue
            }
            3 => {
                let v = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
                if R == 3 {
                    core.registers.set_af(v)
                } else {
                    match R {
                        0 => core.registers.set_bc(v),
                        1 => core.registers.set_de(v),
                        2 => core.registers.set_hl(v),
                        _ => {}
                    }
                }
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}
