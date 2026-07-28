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
    ((core.operands[1] as u16) << 8) | core.operands[0] as u16
}
fn t3(c: &Lr35902Cpu) -> bool {
    c.t_cycle == 2
}
fn t4(c: &Lr35902Cpu) -> bool {
    c.t_cycle == 3
}

// ── LD r16, d16 (3 M-cycles) ──────────────────────────────
// step=0 T4: Continue. step=1 T3: read lo, T4: Continue. step=2 T3: read hi, T4: write+Exit.

pub(crate) struct LdR16D16<const R: u8>;
impl<const R: u8> CpuStepState for LdR16D16<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            if step == 1 {
                core.operands[0] = core.pc_read(bus);
            }
            if step == 2 {
                core.operands[1] = core.pc_read(bus);
            }
        } else if t4(core) {
            if step == 1 {
                return StepResult::Continue;
            }
            if step == 2 {
                write_r16(core, R, addr16(core));
                return StepResult::Exit;
            }
        }
        StepResult::Continue
    }
}

// ── LD (BC/DE), A (2 M-cycles) ──────────────────────────

pub(crate) struct LdR16memA<const R: u8>;
impl<const R: u8> CpuStepState for LdR16memA<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            bus.write(read_r16(core, R), core.registers.a);
        } else if t4(core) {
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}

// ── LD A, (BC/DE) (2 M-cycles) ──────────────────────────

pub(crate) struct LdAR16mem<const R: u8>;
impl<const R: u8> CpuStepState for LdAR16mem<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            core.operands[0] = bus.read(read_r16(core, R));
        } else if t4(core) {
            core.registers.a = core.operands[0];
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}

// ── LD r8, d8 (2 M-cycles) ────────────────────────────────

pub(crate) struct LdR8D8<const R: u8>;
impl<const R: u8> CpuStepState for LdR8D8<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            core.operands[0] = core.pc_read(bus);
        } else if t4(core) {
            write_r8(core, R, core.operands[0]);
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}

// ── LD r8, r8 (1-2 M-cycles) ──────────────────────────────

pub(crate) struct LdR8R8;
impl CpuStepState for LdR8R8 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let op = core.opcode;
        let src = r8(op);
        let dst = (op >> 3) & 0x07;
        let hl_src = src == R8_HL;
        let hl_dst = dst == R8_HL;
        if hl_src || hl_dst {
            // 2 M-cycles
            if step == 0 {
                return StepResult::Continue;
            }
            if t3(core) {
                if hl_src {
                    core.operands[0] = bus.read(core.registers.hl());
                } else {
                    let v = read_r8(core, src);
                    bus.write(core.registers.hl(), v);
                }
            } else if t4(core) {
                if hl_src {
                    write_r8(core, dst, core.operands[0]);
                }
                return StepResult::Exit;
            }
        } else {
            // 1 M-cycle
            if step == 0 {
                write_r8(core, dst, read_r8(core, src));
                return StepResult::Exit;
            }
        }
        StepResult::Continue
    }
}

// ── LD (HL+), A / LD A, (HL+) ──────────────────────────────

pub(crate) struct LdHliA;
impl CpuStepState for LdHliA {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            bus.write(core.registers.hl(), core.registers.a);
        } else if t4(core) {
            core.registers.set_hl(core.registers.hl().wrapping_add(1));
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct LdAHli;
impl CpuStepState for LdAHli {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            core.operands[0] = bus.read(core.registers.hl());
        } else if t4(core) {
            core.registers.a = core.operands[0];
            core.registers.set_hl(core.registers.hl().wrapping_add(1));
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}

// ── LD (HL-), A / LD A, (HL-) ──────────────────────────────

pub(crate) struct LdHldA;
impl CpuStepState for LdHldA {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            bus.write(core.registers.hl(), core.registers.a);
        } else if t4(core) {
            core.registers.set_hl(core.registers.hl().wrapping_sub(1));
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
pub(crate) struct LdAHld;
impl CpuStepState for LdAHld {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            core.operands[0] = bus.read(core.registers.hl());
        } else if t4(core) {
            core.registers.a = core.operands[0];
            core.registers.set_hl(core.registers.hl().wrapping_sub(1));
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}

// ── LD (HL), d8 (3 M-cycles) ───────────────────────────────

pub(crate) struct LdHlD8;
impl CpuStepState for LdHlD8 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) {
            if step == 1 {
                core.operands[0] = core.pc_read(bus);
            }
            if step == 2 {
                bus.write(core.registers.hl(), core.operands[0]);
            }
        } else if t4(core) {
            if step == 1 {
                return StepResult::Continue;
            }
            if step == 2 {
                return StepResult::Exit;
            }
        }
        StepResult::Continue
    }
}

// ── LD (a16), SP (5 M-cycles) ──────────────────────────────

pub(crate) struct LdA16Sp;
impl CpuStepState for LdA16Sp {
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
                3 => {
                    let addr = addr16(core);
                    bus.write(addr, core.registers.sp as u8);
                    core.operands[0] = addr as u8;
                    core.operands[1] = (addr >> 8) as u8;
                }
                4 => {
                    let addr = (core.operands[1] as u16) << 8 | core.operands[0] as u16;
                    bus.write(addr.wrapping_add(1), (core.registers.sp >> 8) as u8);
                }
                _ => unreachable!(),
            }
        } else if t4(core) {
            match step {
                1..=3 => return StepResult::Continue,
                4 => return StepResult::Exit,
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

// ── LD (a16), A (4 M-cycles) ───────────────────────────────

pub(crate) struct LdA16A;
impl CpuStepState for LdA16A {
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
                3 => {
                    bus.write(addr16(core), core.registers.a);
                }
                _ => unreachable!(),
            }
        } else if t4(core) {
            match step {
                1 | 2 => return StepResult::Continue,
                3 => return StepResult::Exit,
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

// ── LD A, (a16) (4 M-cycles) ───────────────────────────────

pub(crate) struct LdAA16;
impl CpuStepState for LdAA16 {
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
                3 => {
                    core.operands[0] = bus.read(addr16(core));
                }
                _ => unreachable!(),
            }
        } else if t4(core) {
            match step {
                1 | 2 => return StepResult::Continue,
                3 => {
                    core.registers.a = core.operands[0];
                    return StepResult::Exit;
                }
                _ => unreachable!(),
            }
        }
        StepResult::Continue
    }
}

// ── LD HL, SP+e (3 M-cycles) ───────────────────────────────

pub(crate) struct LdHlSpE;
impl CpuStepState for LdHlSpE {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if t3(core) && step == 1 {
            core.operands[0] = core.pc_read(bus);
        } else if t4(core) && step == 1 {
            let offset = core.operands[0] as i8;
            let sp = core.registers.sp;
            let r = sp.wrapping_add_signed(offset as i16);
            core.registers
                .set_h((sp & 0x000F) + (offset as u8 as u16 & 0x000F) > 0x000F);
            core.registers
                .set_c((sp & 0x00FF) + (offset as u8 as u16 & 0x00FF) > 0x00FF);
            core.registers.set_z(false);
            core.registers.set_n(false);
            core.registers.set_hl(r);
            return StepResult::Exit;
        }
        StepResult::Continue
    }
}
