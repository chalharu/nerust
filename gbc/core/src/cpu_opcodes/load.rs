use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::cpu_opcodes::helpers::{read_r8, read_r16, write_r8, write_r16};
use crate::memory::GbcMemoryBus;

fn r8(opcode: u8) -> u8 {
    opcode & 0x07
}
const R8_HL: u8 = 6;

fn addr16(core: &Lr35902Cpu) -> u16 {
    ((core.operand(1) as u16) << 8) | core.operand(0) as u16
}

// ── LD r16, d16 (3 M-cycles) ──────────────────────────────

pub(crate) struct LdR16D16<const R: u8>;
impl<const R: u8> CpuStepState for LdR16D16<R> {
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
            write_r16(core, R, addr16(core));
            return StepResult::Exit;
        }
        unreachable!()
    }
}

// ── LD (BC/DE), A (2 M-cycles) ──────────────────────────

pub(crate) struct LdR16memA<const R: u8>;
impl<const R: u8> CpuStepState for LdR16memA<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        bus.write(read_r16(core, R), core.registers().a());
        StepResult::Exit
    }
}

// ── LD A, (BC/DE) (2 M-cycles) ──────────────────────────

pub(crate) struct LdAR16mem<const R: u8>;
impl<const R: u8> CpuStepState for LdAR16mem<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        let addr = read_r16(core, R);
        core.registers_mut().set_a(bus.read(addr));
        StepResult::Exit
    }
}

// ── LD r8, d8 (2 M-cycles) ────────────────────────────────

pub(crate) struct LdR8D8<const R: u8>;
impl<const R: u8> CpuStepState for LdR8D8<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        let v = core.pc_read(bus);
        write_r8(core, R, v);
        StepResult::Exit
    }
}

// ── LD r8, r8 (1-2 M-cycles) ──────────────────────────────

pub(crate) struct LdR8R8;
impl CpuStepState for LdR8R8 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let op = core.opcode();
        let src = r8(op);
        let dst = (op >> 3) & 0x07;
        if src == R8_HL {
            if step == 0 {
                return StepResult::Continue;
            }
            let v = bus.read(core.registers().hl());
            write_r8(core, dst, v);
            StepResult::Exit
        } else if dst == R8_HL {
            if step == 0 {
                return StepResult::Continue;
            }
            bus.write(core.registers().hl(), read_r8(core, src));
            StepResult::Exit
        } else {
            write_r8(core, dst, read_r8(core, src));
            StepResult::Exit
        }
    }
}

// ── LD (HL+), A / LD A, (HL+) ──────────────────────────────

pub(crate) struct LdHliA;
impl CpuStepState for LdHliA {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        bus.write(core.registers().hl(), core.registers().a());
        let _t = core.registers().hl().wrapping_add(1);
        core.registers_mut().set_hl(_t);
        StepResult::Exit
    }
}
pub(crate) struct LdAHli;
impl CpuStepState for LdAHli {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        let a = core.registers().hl();
        bus.trigger_oam_bug(a);
        core.registers_mut().set_a(bus.read(a));
        core.registers_mut().set_hl(a.wrapping_add(1));
        StepResult::Exit
    }
}

// ── LD (HL-), A / LD A, (HL-) ──────────────────────────────

pub(crate) struct LdHldA;
impl CpuStepState for LdHldA {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        bus.write(core.registers().hl(), core.registers().a());
        let _t = core.registers().hl().wrapping_sub(1);
        core.registers_mut().set_hl(_t);
        StepResult::Exit
    }
}
pub(crate) struct LdAHld;
impl CpuStepState for LdAHld {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        let a = core.registers().hl();
        bus.trigger_oam_bug(a);
        core.registers_mut().set_a(bus.read(a));
        core.registers_mut().set_hl(a.wrapping_sub(1));
        StepResult::Exit
    }
}

// ── LD (HL), d8 (3 M-cycles) ───────────────────────────────

pub(crate) struct LdHlD8;
impl CpuStepState for LdHlD8 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            let v = core.pc_read(bus);
            core.set_operand(0, v);
            return StepResult::Continue;
        }
        bus.write(core.registers().hl(), core.operand(0));
        StepResult::Exit
    }
}

// ── LD (a16), SP (5 M-cycles) ──────────────────────────────

pub(crate) struct LdA16Sp;
impl CpuStepState for LdA16Sp {
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
            let addr = addr16(core);
            bus.write(addr, core.registers().sp() as u8);
            core.set_operand(0, addr as u8);
            core.set_operand(1, (addr >> 8) as u8);
            return StepResult::Continue;
        }
        let addr = (core.operand(1) as u16) << 8 | core.operand(0) as u16;
        bus.write(addr.wrapping_add(1), (core.registers().sp() >> 8) as u8);
        StepResult::Exit
    }
}

// ── LD (a16), A (4 M-cycles) ───────────────────────────────

pub(crate) struct LdA16A;
impl CpuStepState for LdA16A {
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
        bus.write(addr16(core), core.registers().a());
        StepResult::Exit
    }
}

// ── LD A, (a16) (4 M-cycles) ───────────────────────────────

pub(crate) struct LdAA16;
impl CpuStepState for LdAA16 {
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
        let addr = addr16(core);
        core.registers_mut().set_a(bus.read(addr));
        StepResult::Exit
    }
}

// ── LD HL, SP+e (3 M-cycles) ───────────────────────────────

pub(crate) struct LdHlSpE;
impl CpuStepState for LdHlSpE {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            let v = core.pc_read(bus);
            core.set_operand(0, v);
            return StepResult::Continue;
        }
        let offset = core.operand(0) as i8;
        let sp = core.registers().sp();
        let r = sp.wrapping_add_signed(offset as i16);
        core.registers_mut()
            .set_h_flag((sp & 0x000F) + (offset as u8 as u16 & 0x000F) > 0x000F);
        core.registers_mut()
            .set_c_flag((sp & 0x00FF) + (offset as u8 as u16 & 0x00FF) > 0x00FF);
        core.registers_mut().set_z(false);
        core.registers_mut().set_n(false);
        core.registers_mut().set_hl(r);
        StepResult::Exit
    }
}
