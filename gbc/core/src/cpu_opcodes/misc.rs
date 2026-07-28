use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::memory::GbcMemoryBus;

pub(crate) struct Nop;
impl CpuStepState for Nop {
    fn exec(_: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}

pub(crate) struct Rlca;
impl CpuStepState for Rlca {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            let c = core.registers.a & 0x80 != 0;
            core.registers.a = (core.registers.a << 1) | c as u8;
            core.registers.set_z(false);
            core.registers.set_n(false);
            core.registers.set_h(false);
            core.registers.set_c(c);
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct Rla;
impl CpuStepState for Rla {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            let c = core.registers.a & 0x80 != 0;
            core.registers.a = (core.registers.a << 1) | core.registers.c_flag() as u8;
            core.registers.set_z(false);
            core.registers.set_n(false);
            core.registers.set_h(false);
            core.registers.set_c(c);
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct Rrca;
impl CpuStepState for Rrca {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            let c = core.registers.a & 0x01 != 0;
            core.registers.a = (core.registers.a >> 1) | (if c { 0x80 } else { 0 });
            core.registers.set_z(false);
            core.registers.set_n(false);
            core.registers.set_h(false);
            core.registers.set_c(c);
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct Rra;
impl CpuStepState for Rra {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            let c = core.registers.a & 0x01 != 0;
            core.registers.a =
                (core.registers.a >> 1) | (if core.registers.c_flag() { 0x80 } else { 0 });
            core.registers.set_z(false);
            core.registers.set_n(false);
            core.registers.set_h(false);
            core.registers.set_c(c);
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct Daa;
impl CpuStepState for Daa {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            let mut a = core.registers.a;
            let n = core.registers.n_flag();
            let c = core.registers.c_flag();
            let h = core.registers.h_flag();
            if !n {
                if c || a > 0x99 {
                    a = a.wrapping_add(0x60);
                    core.registers.set_c(true);
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
            core.registers.set_z(a == 0);
            core.registers.set_h(false);
            core.registers.a = a;
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct Cpl;
impl CpuStepState for Cpl {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            core.registers.a = !core.registers.a;
            core.registers.set_n(true);
            core.registers.set_h(true);
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct Scf;
impl CpuStepState for Scf {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            core.registers.set_n(false);
            core.registers.set_h(false);
            core.registers.set_c(true);
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct Ccf;
impl CpuStepState for Ccf {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            let c = core.registers.c_flag();
            core.registers.set_n(false);
            core.registers.set_h(false);
            core.registers.set_c(!c);
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct Invalid;
impl CpuStepState for Invalid {
    fn exec(_: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}

pub(crate) struct Halt;
impl CpuStepState for Halt {
    fn exec(_: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            bus.halt_cpu();
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct Stop;
impl CpuStepState for Stop {
    fn exec(_: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            bus.stop();
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct Ei;
impl CpuStepState for Ei {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            core.ime_delayed = true;
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct Di;
impl CpuStepState for Di {
    fn exec(_: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            bus.set_ime(false);
            return StepResult::Exit;
        }
        StepResult::Continue
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
            core.operands[0] = core.pc_read(bus);
            return StepResult::Continue;
        }
        bus.write(0xFF00 | core.operands[0] as u16, core.registers.a);
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
            core.operands[0] = core.pc_read(bus);
            return StepResult::Continue;
        }
        core.registers.a = bus.read(0xFF00 | core.operands[0] as u16);
        StepResult::Exit
    }
}
pub(crate) struct LdCA;
impl CpuStepState for LdCA {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        bus.write(0xFF00 | core.registers.c as u16, core.registers.a);
        StepResult::Exit
    }
}
pub(crate) struct LdAC;
impl CpuStepState for LdAC {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        core.registers.a = bus.read(0xFF00 | core.registers.c as u16);
        StepResult::Exit
    }
}
pub(crate) struct LdSpHl;
impl CpuStepState for LdSpHl {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        core.registers.sp = core.registers.hl();
        StepResult::Exit
    }
}
