//! CB-prefix instructions: shift/rotate/bit/res/set.

use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::memory::GbcMemoryBus;

pub(crate) struct CbPrefix;
impl CpuStepState for CbPrefix {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        match step {
            1 => StepResult::Continue,
            2 => {
                core.operands[0] = core.pc_read(bus);
                let idx = core.operands[0] & 0x07;
                if idx != 6 {
                    cb_exec_reg(core, bus);
                    StepResult::Exit
                } else {
                    StepResult::Continue
                }
            }
            3 => {
                let op = core.operands[0];
                let val = bus.read(core.registers.hl());
                let cat = op >> 6;
                if cat == 1 {
                    let bit = (op >> 3) & 0x07;
                    core.registers.set_z(val & (1 << bit) == 0);
                    core.registers.set_n(false);
                    core.registers.set_h(true);
                    StepResult::Exit
                } else {
                    core.operands[1] = cb_exec_val(val, op, core);
                    StepResult::Continue
                }
            }
            4 => {
                let v = core.operands[1];
                bus.write(core.registers.hl(), v);
                StepResult::Exit
            }
            _ => unreachable!(),
        }
    }
}

fn cb_exec_reg(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus) {
    let op = core.operands[0];
    let idx = op & 0x07;
    let val = match idx {
        0 => core.registers.b,
        1 => core.registers.c,
        2 => core.registers.d,
        3 => core.registers.e,
        4 => core.registers.h,
        5 => core.registers.l,
        7 => core.registers.a,
        _ => 0,
    };
    let cat = op >> 6;
    match cat {
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
            match idx {
                0 => core.registers.b = r,
                1 => core.registers.c = r,
                2 => core.registers.d = r,
                3 => core.registers.e = r,
                4 => core.registers.h = r,
                5 => core.registers.l = r,
                7 => core.registers.a = r,
                _ => {}
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
        _ => {
            let bit = (op >> 3) & 0x07;
            let v = val | (1 << bit);
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
    }
}

fn cb_exec_val(val: u8, op: u8, core: &mut Lr35902Cpu) -> u8 {
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
            r
        }
        1 => {
            let bit = (op >> 3) & 0x07;
            core.registers.set_z(val & (1 << bit) == 0);
            core.registers.set_n(false);
            core.registers.set_h(true);
            val
        }
        2 => {
            let bit = (op >> 3) & 0x07;
            val & !(1 << bit)
        }
        _ => {
            let bit = (op >> 3) & 0x07;
            val | (1 << bit)
        }
    }
}
