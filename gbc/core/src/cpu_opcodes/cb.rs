//! CB-prefix instructions: shift/rotate/bit/res/set.

use crate::cpu_core::Lr35902Cpu;
use crate::cpu_opcodes::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::memory::GbcMemoryBus;

pub(crate) struct CbPrefix;
impl CpuStepState for CbPrefix {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => {
                core.operands[0] = core.pc_read(bus);
                StepResult::Continue
            }
            2 => {
                cb_exec(core, bus);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

fn reg_idx(r: u8) -> u8 {
    r & 0x07
}

fn read_r(core: &Lr35902Cpu, idx: u8) -> u8 {
    match idx {
        0 => core.registers.b,
        1 => core.registers.c,
        2 => core.registers.d,
        3 => core.registers.e,
        4 => core.registers.h,
        5 => core.registers.l,
        7 => core.registers.a,
        _ => 0,
    }
}
fn write_r(core: &mut Lr35902Cpu, idx: u8, v: u8) {
    match idx {
        0 => core.registers.b = v,
        1 => core.registers.c = v,
        2 => core.registers.d = v,
        3 => core.registers.e = v,
        4 => core.registers.h = v,
        5 => core.registers.l = v,
        7 => core.registers.a = v,
        _ => {}
    }
}

fn cb_exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus) {
    let op = core.operands[0];
    let idx = reg_idx(op);
    let is_hl = idx == 6;
    let val = if is_hl {
        bus.read(core.registers.hl())
    } else {
        read_r(core, idx)
    };

    match op >> 6 {
        0 => {
            let op3 = (op >> 3) & 0x07;
            let (r, c) = match op3 {
                0 => {
                    let c = val & 0x80 != 0;
                    ((val << 1) | c as u8, c)
                }
                1 => {
                    let c = val & 0x01 != 0;
                    ((val >> 1) | if c { 0x80 } else { 0 }, c)
                }
                2 => {
                    let c = val & 0x80 != 0;
                    ((val << 1) | core.registers.c_flag() as u8, c)
                }
                3 => {
                    let c = val & 0x01 != 0;
                    (
                        (val >> 1) | if core.registers.c_flag() { 0x80 } else { 0 },
                        c,
                    )
                }
                4 => {
                    let c = val & 0x80 != 0;
                    (val << 1, c)
                }
                5 => {
                    let c = val & 0x01 != 0;
                    ((val >> 1) | (val & 0x80), c)
                }
                6 => (val.rotate_right(4), false),
                _ => {
                    let c = val & 0x01 != 0;
                    (val >> 1, c)
                }
            };
            core.registers.set_z(r == 0);
            core.registers.set_n(false);
            core.registers.set_h(false);
            core.registers.set_c(c);
            if is_hl {
                bus.write(core.registers.hl(), r);
            } else {
                write_r(core, idx, r);
            }
        }
        1 => {
            let bit = (op >> 3) & 0x07;
            core.registers.set_z(val & (1 << bit) == 0);
            core.registers.set_n(false);
            core.registers.set_h(true);
        }
        2 => {
            let bit = (op >> 3) & 0x07;
            let v = val & !(1 << bit);
            if is_hl {
                bus.write(core.registers.hl(), v);
            } else {
                write_r(core, idx, v);
            }
        }
        _ => {
            let bit = (op >> 3) & 0x07;
            let v = val | (1 << bit);
            if is_hl {
                bus.write(core.registers.hl(), v);
            } else {
                write_r(core, idx, v);
            }
        }
    }
}
