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

fn jump16(core: &mut Lr35902Cpu) {
    core.registers
        .set_pc(((core.operand(1) as u16) << 8) | core.operand(0) as u16);
}

// ── JP a16 (4 M-cycles) ────────────────────────────────────

pub(crate) struct JpA16;
impl CpuStepState for JpA16 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            let v = core.pc_read(bus);
            core.set_operand(0, v);
            return StepResult::Continue;
        }
        if step == 2 {
            let v = core.pc_read(bus);
            core.set_operand(1, v);
            return StepResult::Continue;
        }
        debug_assert!(step == 3, "JP step > 3");
        jump16(core);
        StepResult::Exit
    }
}

pub(crate) struct JpCond<const C: u8>;
impl<const C: u8> CpuStepState for JpCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            let v = core.pc_read(bus);
            core.set_operand(0, v);
            return StepResult::Continue;
        }
        if step == 2 {
            let v = core.pc_read(bus);
            core.set_operand(1, v);
            if !cond(C, core) {
                return StepResult::Exit;
            }
            return StepResult::Continue;
        }
        debug_assert!(step == 3, "JP step > 3");
        jump16(core);
        StepResult::Exit
    }
}

pub(crate) struct JpHl;
impl CpuStepState for JpHl {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        core.registers.set_pc(core.registers.hl());
        StepResult::Exit
    }
}

// ── JR e (3 M-cycles) ──────────────────────────────────────

pub(crate) struct Jr;
impl CpuStepState for Jr {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            let v = core.pc_read(bus);
            core.set_operand(0, v);
            return StepResult::Continue;
        }
        core.registers.set_pc(
            core.registers
                .pc()
                .wrapping_add_signed(core.operand(0) as i8 as i16),
        );
        StepResult::Exit
    }
}

pub(crate) struct JrCond<const C: u8>;
impl<const C: u8> CpuStepState for JrCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let taken = cond(C, core);
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            let v = core.pc_read(bus);
            core.set_operand(0, v);
            if !taken {
                return StepResult::Exit;
            }
            return StepResult::Continue;
        }
        debug_assert!(step == 2, "JR step > 2");
        core.registers.set_pc(
            core.registers
                .pc()
                .wrapping_add_signed(core.operand(0) as i8 as i16),
        );
        StepResult::Exit
    }
}

// ── CALL (6 M-cycles) ──────────────────────────────────────

pub(crate) struct Call;
impl CpuStepState for Call {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            let v = core.pc_read(bus);
            core.set_operand(0, v);
            return StepResult::Continue;
        }
        if step == 2 {
            let v = core.pc_read(bus);
            core.set_operand(1, v);
            return StepResult::Continue;
        }
        if step == 3 {
            return StepResult::Continue;
        }
        if step == 4 {
            core.registers.set_sp(core.registers.sp().wrapping_sub(1));
            bus.write(core.registers.sp(), (core.registers.pc() >> 8) as u8);
            return StepResult::Continue;
        }
        core.registers.set_sp(core.registers.sp().wrapping_sub(1));
        bus.write(core.registers.sp(), core.registers.pc() as u8);
        jump16(core);
        StepResult::Exit
    }
}

pub(crate) struct CallCond<const C: u8>;
impl<const C: u8> CpuStepState for CallCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let taken = cond(C, core);
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            let v = core.pc_read(bus);
            core.set_operand(0, v);
            return StepResult::Continue;
        }
        if step == 2 {
            let v = core.pc_read(bus);
            core.set_operand(1, v);
            if !taken {
                return StepResult::Exit;
            }
            return StepResult::Continue;
        }
        if step == 3 {
            return StepResult::Continue;
        }
        if step == 4 {
            core.registers.set_sp(core.registers.sp().wrapping_sub(1));
            bus.write(core.registers.sp(), (core.registers.pc() >> 8) as u8);
            return StepResult::Continue;
        }
        core.registers.set_sp(core.registers.sp().wrapping_sub(1));
        bus.write(core.registers.sp(), core.registers.pc() as u8);
        jump16(core);
        StepResult::Exit
    }
}

// ── RET (4 M-cycles) ───────────────────────────────────────

fn ret_finish(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, set_ime: bool) -> StepResult {
    if set_ime {
        bus.set_ime(true);
    }
    let v = bus.read(core.registers.sp());
    core.set_operand(1, v);
    core.registers.set_sp(core.registers.sp().wrapping_add(1));
    jump16(core);
    StepResult::Exit
}

fn ret_common(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
    if step == 0 {
        return StepResult::Continue;
    }
    if step == 1 {
        return StepResult::Continue;
    }
    debug_assert!(step == 2, "RET step > 2");
    let v = bus.read(core.registers.sp());
    core.set_operand(0, v);
    core.registers.set_sp(core.registers.sp().wrapping_add(1));
    StepResult::Continue
}

pub(crate) struct Ret;
impl CpuStepState for Ret {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step < 3 {
            return ret_common(core, bus, step);
        }
        ret_finish(core, bus, false)
    }
}

pub(crate) struct RetCond<const C: u8>;
impl<const C: u8> CpuStepState for RetCond<C> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let taken = cond(C, core);
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            return if taken {
                StepResult::Continue
            } else {
                StepResult::Exit
            };
        }
        if step == 2 {
            let v = bus.read(core.registers.sp());
            core.set_operand(0, v);
            core.registers.set_sp(core.registers.sp().wrapping_add(1));
            return StepResult::Continue;
        }
        if step == 3 {
            return StepResult::Continue;
        }
        ret_finish(core, bus, false)
    }
}

pub(crate) struct Reti;
impl CpuStepState for Reti {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step < 3 {
            return ret_common(core, bus, step);
        }
        ret_finish(core, bus, true)
    }
}

// ── RST (4 M-cycles) ───────────────────────────────────────

pub(crate) struct Rst<const V: u8>;
impl<const V: u8> CpuStepState for Rst<V> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            return StepResult::Continue;
        }
        if step == 2 {
            core.registers.set_sp(core.registers.sp().wrapping_sub(1));
            bus.write(core.registers.sp(), (core.registers.pc() >> 8) as u8);
            return StepResult::Continue;
        }
        core.registers.set_sp(core.registers.sp().wrapping_sub(1));
        bus.write(core.registers.sp(), core.registers.pc() as u8);
        core.registers.set_pc(V as u16 * 8);
        StepResult::Exit
    }
}
