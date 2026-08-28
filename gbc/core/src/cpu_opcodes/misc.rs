use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::memory::GbcMemoryBus;

pub(crate) struct Nop;
impl CpuStepState for Nop {
    fn exec(_: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        StepResult::Exit
    }
}

pub(crate) struct Rlca;
impl CpuStepState for Rlca {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let c = core.registers().a() & 0x80 != 0;
        let a_val = core.registers().a();
        core.registers_mut().set_a((a_val << 1) | c as u8);
        core.registers_mut().set_z(false);
        core.registers_mut().set_n(false);
        core.registers_mut().set_h_flag(false);
        core.registers_mut().set_c_flag(c);
        StepResult::Exit
    }
}
pub(crate) struct Rla;
impl CpuStepState for Rla {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let c = core.registers().a() & 0x80 != 0;
        let a_val = core.registers().a();
        let cf = core.registers().c_flag() as u8;
        core.registers_mut().set_a((a_val << 1) | cf);
        core.registers_mut().set_z(false);
        core.registers_mut().set_n(false);
        core.registers_mut().set_h_flag(false);
        core.registers_mut().set_c_flag(c);
        StepResult::Exit
    }
}
pub(crate) struct Rrca;
impl CpuStepState for Rrca {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let c = core.registers().a() & 0x01 != 0;
        let a_val = core.registers().a();
        core.registers_mut()
            .set_a((a_val >> 1) | (if c { 0x80 } else { 0 }));
        core.registers_mut().set_z(false);
        core.registers_mut().set_n(false);
        core.registers_mut().set_h_flag(false);
        core.registers_mut().set_c_flag(c);
        StepResult::Exit
    }
}
pub(crate) struct Rra;
impl CpuStepState for Rra {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let c = core.registers().a() & 0x01 != 0;
        let a_val = core.registers().a();
        let cf = core.registers().c_flag();
        core.registers_mut()
            .set_a((a_val >> 1) | (if cf { 0x80 } else { 0 }));
        core.registers_mut().set_z(false);
        core.registers_mut().set_n(false);
        core.registers_mut().set_h_flag(false);
        core.registers_mut().set_c_flag(c);
        StepResult::Exit
    }
}
pub(crate) struct Daa;
impl CpuStepState for Daa {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let mut a = core.registers().a();
        let n = core.registers().n_flag();
        let c = core.registers().c_flag();
        let h = core.registers().h_flag();
        if !n {
            if c || a > 0x99 {
                a = a.wrapping_add(0x60);
                core.registers_mut().set_c_flag(true);
            }
            if h || (a & 0x0F) > 0x09 {
                a = a.wrapping_add(0x06);
            }
        } else {
            if c {
                a = a.wrapping_sub(0x60);
            }
            if h {
                a = a.wrapping_sub(0x06);
            }
        }
        core.registers_mut().set_z(a == 0);
        core.registers_mut().set_h_flag(false);
        core.registers_mut().set_a(a);
        StepResult::Exit
    }
}
pub(crate) struct Cpl;
impl CpuStepState for Cpl {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let _t = !core.registers().a();
        core.registers_mut().set_a(_t);
        core.registers_mut().set_n(true);
        core.registers_mut().set_h_flag(true);
        StepResult::Exit
    }
}
pub(crate) struct Scf;
impl CpuStepState for Scf {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        core.registers_mut().set_n(false);
        core.registers_mut().set_h_flag(false);
        core.registers_mut().set_c_flag(true);
        StepResult::Exit
    }
}
pub(crate) struct Ccf;
impl CpuStepState for Ccf {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        let c = core.registers().c_flag();
        core.registers_mut().set_n(false);
        core.registers_mut().set_h_flag(false);
        core.registers_mut().set_c_flag(!c);
        StepResult::Exit
    }
}
pub(crate) struct Invalid;
impl CpuStepState for Invalid {
    fn exec(_: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        StepResult::Exit
    }
}

pub(crate) struct Halt;
impl CpuStepState for Halt {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        // EI immediately before HALT: the delayed IME must take effect for
        // the HALT to wait for interrupts (real hardware enables IME during
        // the instruction after EI).
        if core.take_armed_ime() {
            bus.set_ime(true);
            if bus.interrupt_pending() {
                let pc = core.registers().pc();
                core.registers_mut().set_pc(pc.wrapping_sub(1));
            }
        }
        bus.halt_cpu();
        StepResult::Exit
    }
}
pub(crate) struct Stop;
impl CpuStepState for Stop {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        let _padding = core.pc_read(bus);
        bus.stop();
        StepResult::Exit
    }
}
pub(crate) struct Ei;
impl CpuStepState for Ei {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, _step: u8) -> StepResult {
        core.set_ime_delayed(true);
        StepResult::Exit
    }
}
pub(crate) struct Di;
impl CpuStepState for Di {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, _step: u8) -> StepResult {
        core.cancel_delayed_ime();
        bus.set_ime(false);
        StepResult::Exit
    }
}
pub(crate) struct InvalidOp<const STEP: u8, const M: u8>;
impl<const STEP: u8, const M: u8> CpuStepState for InvalidOp<STEP, M> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        core.pc_read(bus);
        if step >= M {
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}

pub(crate) struct LdhA8A;
impl CpuStepState for LdhA8A {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            let v = core.pc_read(bus);
            core.set_operand(0, v);
            return StepResult::Continue;
        }
        bus.write(0xFF00 | core.operand(0) as u16, core.registers().a());
        StepResult::Exit
    }
}
pub(crate) struct LdhAA8;
impl CpuStepState for LdhAA8 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            let v = core.pc_read(bus);
            core.set_operand(0, v);
            return StepResult::Continue;
        }
        let op0 = core.operand(0) as u16;
        core.registers_mut().set_a(bus.read(0xFF00 | op0));
        StepResult::Exit
    }
}
pub(crate) struct LdCA;
impl CpuStepState for LdCA {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        bus.write(0xFF00 | core.registers().c() as u16, core.registers().a());
        StepResult::Exit
    }
}
pub(crate) struct LdAC;
impl CpuStepState for LdAC {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        let c_val = core.registers().c() as u16;
        core.registers_mut().set_a(bus.read(0xFF00 | c_val));
        StepResult::Exit
    }
}
pub(crate) struct LdSpHl;
impl CpuStepState for LdSpHl {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        {
            let hl = core.registers().hl();
            core.registers_mut().set_sp(hl)
        };
        StepResult::Exit
    }
}
