use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::memory::GbcMemoryBus;

fn t3(c: &Lr35902Cpu) -> bool {
    c.t_cycle == 2
}
fn t4(c: &Lr35902Cpu) -> bool {
    c.t_cycle == 3
}

fn cond(c: u8, core: &Lr35902Cpu) -> bool {
    match c {
        0 => !core.registers.z_flag(),
        1 => core.registers.z_flag(),
        2 => !core.registers.c_flag(),
        _ => core.registers.c_flag(),
    }
}

fn jump16(core: &mut Lr35902Cpu) {
    core.registers.pc = ((core.operands[1] as u16) << 8) | core.operands[0] as u16;
}

// ── JP a16 (4 M-cycles) ────────────────────────────────────

pub(crate) struct JpA16;
impl CpuStepState for JpA16 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            match step {
                1 => {
                    core.operands[0] = core.pc_read(bus);
                }
                2 => {
                    core.operands[1] = core.pc_read(bus);
                }
                _ => {}
            }
        } else if t4(core) {
            match step {
                1 | 2 => return StepResult::Continue,
                3 => {
                    jump16(core);
                    return StepResult::Exit;
                }
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

pub(crate) struct JpCond<const C: u8>;
impl<const C: u8> CpuStepState for JpCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            match step {
                1 => {
                    core.operands[0] = core.pc_read(bus);
                }
                2 => {
                    core.operands[1] = core.pc_read(bus);
                }
                _ => {}
            }
        } else if t4(core) {
            match step {
                1 => return StepResult::Continue,
                2 => {
                    if !cond(C, core) {
                        return StepResult::Exit;
                    }
                    return StepResult::Continue;
                }
                3 => {
                    jump16(core);
                    return StepResult::Exit;
                }
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

pub(crate) struct JpHl;
impl CpuStepState for JpHl {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            core.registers.pc = core.registers.hl();
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}

// ── JR e (3 M-cycles) ──────────────────────────────────────

pub(crate) struct Jr;
impl CpuStepState for Jr {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) && step == 1 {
            core.operands[0] = core.pc_read(bus);
        } else if t4(core) && step == 1 {
            core.registers.pc = core
                .registers
                .pc
                .wrapping_add_signed(core.operands[0] as i8 as i16);
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}

pub(crate) struct JrCond<const C: u8>;
impl<const C: u8> CpuStepState for JrCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let taken = cond(C, core);
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            if step == 1 {
                core.operands[0] = core.pc_read(bus);
            }
        } else if t4(core) {
            match step {
                1 => {
                    if !taken {
                        return StepResult::Exit;
                    }
                    return StepResult::Continue;
                }
                2 => {
                    core.registers.pc = core
                        .registers
                        .pc
                        .wrapping_add_signed(core.operands[0] as i8 as i16);
                    return StepResult::Exit;
                }
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

// ── CALL (6 M-cycles) ──────────────────────────────────────

pub(crate) struct Call;
impl CpuStepState for Call {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            match step {
                1 => {
                    core.operands[0] = core.pc_read(bus);
                }
                2 => {
                    core.operands[1] = core.pc_read(bus);
                }
                4 => {
                    core.registers.sp = core.registers.sp.wrapping_sub(1);
                    bus.write(core.registers.sp, (core.registers.pc >> 8) as u8);
                }
                5 => {
                    core.registers.sp = core.registers.sp.wrapping_sub(1);
                    bus.write(core.registers.sp, core.registers.pc as u8);
                }
                _ => {}
            }
        } else if t4(core) {
            match step {
                1..=4 => return StepResult::Continue,
                5 => {
                    jump16(core);
                    return StepResult::Exit;
                }
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

pub(crate) struct CallCond<const C: u8>;
impl<const C: u8> CpuStepState for CallCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let taken = cond(C, core);
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            match step {
                1 => {
                    core.operands[0] = core.pc_read(bus);
                }
                2 => {
                    core.operands[1] = core.pc_read(bus);
                }
                4 => {
                    core.registers.sp = core.registers.sp.wrapping_sub(1);
                    bus.write(core.registers.sp, (core.registers.pc >> 8) as u8);
                }
                5 => {
                    core.registers.sp = core.registers.sp.wrapping_sub(1);
                    bus.write(core.registers.sp, core.registers.pc as u8);
                }
                _ => {}
            }
        } else if t4(core) {
            match step {
                1 => return StepResult::Continue,
                2 => {
                    if !taken {
                        return StepResult::Exit;
                    }
                    return StepResult::Continue;
                }
                3 => return StepResult::Continue,
                4 => return StepResult::Continue,
                5 => {
                    jump16(core);
                    return StepResult::Exit;
                }
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

// ── RET (4 M-cycles) ───────────────────────────────────────

pub(crate) struct Ret;
impl CpuStepState for Ret {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            match step {
                2 => {
                    core.operands[0] = bus.read(core.registers.sp);
                    core.registers.sp = core.registers.sp.wrapping_add(1);
                }
                3 => {
                    core.operands[1] = bus.read(core.registers.sp);
                    core.registers.sp = core.registers.sp.wrapping_add(1);
                }
                _ => {}
            }
        } else if t4(core) {
            match step {
                1 => return StepResult::Continue,
                2 => return StepResult::Continue,
                3 => {
                    jump16(core);
                    return StepResult::Exit;
                }
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

pub(crate) struct RetCond<const C: u8>;
impl<const C: u8> CpuStepState for RetCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let taken = cond(C, core);
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            match step {
                3 => {
                    core.operands[0] = bus.read(core.registers.sp);
                    core.registers.sp = core.registers.sp.wrapping_add(1);
                }
                4 => {
                    core.operands[1] = bus.read(core.registers.sp);
                    core.registers.sp = core.registers.sp.wrapping_add(1);
                }
                _ => {}
            }
        } else if t4(core) {
            match step {
                1 => {
                    if !taken {
                        return StepResult::Exit;
                    }
                    return StepResult::Continue;
                }
                2 => return StepResult::Continue,
                3 => return StepResult::Continue,
                4 => {
                    jump16(core);
                    return StepResult::Exit;
                }
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

pub(crate) struct Reti;
impl CpuStepState for Reti {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            match step {
                2 => {
                    core.operands[0] = bus.read(core.registers.sp);
                    core.registers.sp = core.registers.sp.wrapping_add(1);
                }
                3 => {
                    core.operands[1] = bus.read(core.registers.sp);
                    core.registers.sp = core.registers.sp.wrapping_add(1);
                }
                _ => {}
            }
        } else if t4(core) {
            match step {
                1 => return StepResult::Continue,
                2 => return StepResult::Continue,
                3 => {
                    bus.set_ime(true);
                    jump16(core);
                    return StepResult::Exit;
                }
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

// ── RST (4 M-cycles) ───────────────────────────────────────

pub(crate) struct Rst<const V: u8>;
impl<const V: u8> CpuStepState for Rst<V> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            match step {
                2 => {
                    core.registers.sp = core.registers.sp.wrapping_sub(1);
                    bus.write(core.registers.sp, (core.registers.pc >> 8) as u8);
                }
                3 => {
                    core.registers.sp = core.registers.sp.wrapping_sub(1);
                    bus.write(core.registers.sp, core.registers.pc as u8);
                }
                _ => {}
            }
        } else if t4(core) {
            match step {
                1 => return StepResult::Continue,
                2 => return StepResult::Continue,
                3 => {
                    core.registers.pc = V as u16 * 8;
                    return StepResult::Exit;
                }
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}
