use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::cpu_opcodes::helpers::reg;
use crate::memory::GbcMemoryBus;

fn t3(c: &Lr35902Cpu) -> bool {
    c.t_cycle == 2
}
fn t4(c: &Lr35902Cpu) -> bool {
    c.t_cycle == 3
}

pub(crate) struct Push<const R: u8>;
impl<const R: u8> CpuStepState for Push<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            match step {
                2 => {
                    core.registers.sp = core.registers.sp.wrapping_sub(1);
                    bus.write(core.registers.sp, core.operands[0]);
                }
                3 => {
                    core.registers.sp = core.registers.sp.wrapping_sub(1);
                    bus.write(core.registers.sp, core.operands[1]);
                }
                _ => {}
            }
        } else if t4(core) {
            match step {
                1 => {
                    let v = if R == 3 {
                        core.registers.af()
                    } else {
                        match R {
                            reg::BC => core.registers.bc(),
                            reg::DE => core.registers.de(),
                            reg::R16_HL => core.registers.hl(),
                            _ => 0,
                        }
                    };
                    core.operands[0] = (v >> 8) as u8;
                    core.operands[1] = v as u8;
                    return StepResult::Continue;
                }
                2 => return StepResult::Continue,
                3 => return StepResult::Exit,
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

pub(crate) struct Pop<const R: u8>;
impl<const R: u8> CpuStepState for Pop<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            match step {
                1 => {
                    core.operands[0] = bus.read(core.registers.sp);
                    core.registers.sp = core.registers.sp.wrapping_add(1);
                }
                2 => {
                    core.operands[1] = bus.read(core.registers.sp);
                    core.registers.sp = core.registers.sp.wrapping_add(1);
                }
                _ => {}
            }
        } else if t4(core) {
            match step {
                1 => return StepResult::Continue,
                2 => {
                    let v = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
                    if R == 3 {
                        core.registers.set_af(v)
                    } else {
                        match R {
                            reg::BC => core.registers.set_bc(v),
                            reg::DE => core.registers.set_de(v),
                            reg::R16_HL => core.registers.set_hl(v),
                            _ => {}
                        }
                    }
                    return StepResult::Exit;
                }
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}
