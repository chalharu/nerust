use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::memory::GbcMemoryBus;

fn cond(c: u8, core: &Lr35902Cpu) -> bool {
    match c {
        0 => !core.registers().z_flag(),
        1 => core.registers().z_flag(),
        2 => !core.registers().c_flag(),
        _ => core.registers().c_flag(),
    }
}

fn jump16(core: &mut Lr35902Cpu) {
    let op0 = core.operand(0) as u16;
    let op1 = core.operand(1) as u16;
    core.registers_mut().set_pc((op1 << 8) | op0);
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
        {
            let hl = core.registers().hl();
            core.registers_mut().set_pc(hl)
        };
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
        let pc = core.registers().pc();
        let op0 = core.operand(0) as i8 as i16;
        core.registers_mut().set_pc(pc.wrapping_add_signed(op0));
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
        let pc = core.registers().pc();
        let op0 = core.operand(0) as i8 as i16;
        core.registers_mut().set_pc(pc.wrapping_add_signed(op0));
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
            let _t = core.registers().sp().wrapping_sub(1);
            core.registers_mut().set_sp(_t);
            bus.write(core.registers().sp(), (core.registers().pc() >> 8) as u8);
            return StepResult::Continue;
        }
        let _t = core.registers().sp().wrapping_sub(1);
        core.registers_mut().set_sp(_t);
        bus.write(core.registers().sp(), core.registers().pc() as u8);
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
            let _t = core.registers().sp().wrapping_sub(1);
            core.registers_mut().set_sp(_t);
            bus.write(core.registers().sp(), (core.registers().pc() >> 8) as u8);
            return StepResult::Continue;
        }
        let _t = core.registers().sp().wrapping_sub(1);
        core.registers_mut().set_sp(_t);
        bus.write(core.registers().sp(), core.registers().pc() as u8);
        jump16(core);
        StepResult::Exit
    }
}

// ── RET (4 M-cycles) ───────────────────────────────────────

fn ret_finish(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, set_ime: bool) -> StepResult {
    let v = bus.read(core.registers().sp());
    core.set_operand(1, v);
    let _t = core.registers().sp().wrapping_add(1);
    core.registers_mut().set_sp(_t);
    jump16(core);
    if set_ime {
        bus.set_ime(true);
    }
    StepResult::Continue
}

fn ret_low_byte(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus) -> StepResult {
    let v = bus.read(core.registers().sp());
    core.set_operand(0, v);
    let _t = core.registers().sp().wrapping_add(1);
    core.registers_mut().set_sp(_t);
    StepResult::Continue
}

pub(crate) struct Ret;
impl CpuStepState for Ret {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            return ret_low_byte(core, bus);
        }
        if step == 2 {
            return ret_finish(core, bus, false);
        }
        debug_assert!(step == 3, "RET step > 3");
        StepResult::Exit
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
            return ret_low_byte(core, bus);
        }
        if step == 3 {
            return ret_finish(core, bus, false);
        }
        debug_assert!(step == 4, "RET CC step > 4");
        StepResult::Exit
    }
}

pub(crate) struct Reti;
impl CpuStepState for Reti {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            return ret_low_byte(core, bus);
        }
        if step == 2 {
            return ret_finish(core, bus, true);
        }
        debug_assert!(step == 3, "RETI step > 3");
        StepResult::Exit
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
            let _t = core.registers().sp().wrapping_sub(1);
            core.registers_mut().set_sp(_t);
            bus.write(core.registers().sp(), (core.registers().pc() >> 8) as u8);
            return StepResult::Continue;
        }
        let _t = core.registers().sp().wrapping_sub(1);
        core.registers_mut().set_sp(_t);
        bus.write(core.registers().sp(), core.registers().pc() as u8);
        core.registers_mut().set_pc(V as u16 * 8);
        StepResult::Exit
    }
}
