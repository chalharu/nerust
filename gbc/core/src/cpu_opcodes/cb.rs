use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::memory::GbcMemoryBus;

pub(crate) struct CbPrefix;
impl CpuStepState for CbPrefix {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            return StepResult::Continue;
        }
        if step == 1 {
            core.operands[0] = core.pc_read(bus);
            let idx = core.operands[0] & 0x07;
            if idx != 6 {
                cb_exec_reg(core, bus);
                return StepResult::Exit;
            }
            // (HL) — need more M-cycles
            return StepResult::Continue;
        }
        if step == 2 {
            let op = core.operands[0];
            let idx = op & 0x07;
            debug_assert!(idx == 6, "CB prefix step 2 with idx != 6");
            let val = bus.read(core.registers.hl());
            let cat = op >> 6;
            if cat == 1 {
                // BIT n,(HL)
                let bit = (op >> 3) & 0x07;
                core.registers.set_z(val & (1 << bit) == 0);
                core.registers.set_n(false);
                core.registers.set_h_flag(true);
                return StepResult::Exit;
            }
            // rotate/res/set: compute, need step 3 for write
            core.operands[1] = cb_compute(val, op, core);
            return StepResult::Continue;
        }
        debug_assert!(step == 3, "CB prefix step > 3");
        bus.write(core.registers.hl(), core.operands[1]);
        StepResult::Exit
    }
}

fn read_reg(core: &Lr35902Cpu, idx: u8) -> u8 {
    match idx {
        0 => core.registers.b(),
        1 => core.registers.c(),
        2 => core.registers.d(),
        3 => core.registers.e(),
        4 => core.registers.h(),
        5 => core.registers.l(),
        7 => core.registers.a(),
        _ => 0,
    }
}

fn write_reg(core: &mut Lr35902Cpu, idx: u8, v: u8) {
    match idx {
        0 => core.registers.set_b(v),
        1 => core.registers.set_c(v),
        2 => core.registers.set_d(v),
        3 => core.registers.set_e(v),
        4 => core.registers.set_h(v),
        5 => core.registers.set_l(v),
        7 => core.registers.set_a(v),
        _ => {}
    }
}

fn cb_rotate(val: u8, op: u8, core: &Lr35902Cpu) -> (u8, bool) {
    let op3 = (op >> 3) & 0x07;
    match op3 {
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
    }
}

fn set_rotate_flags(core: &mut Lr35902Cpu, r: u8, c: bool) {
    core.registers.set_z(r == 0);
    core.registers.set_n(false);
    core.registers.set_h_flag(false);
    core.registers.set_c_flag(c);
}

fn cb_exec_reg(core: &mut Lr35902Cpu, _bus: &mut GbcMemoryBus) {
    let op = core.operands[0];
    let idx = op & 0x07;
    let val = read_reg(core, idx);
    let cat = op >> 6;
    match cat {
        0 => {
            let (r, c) = cb_rotate(val, op, core);
            set_rotate_flags(core, r, c);
            write_reg(core, idx, r);
        }
        1 => {
            let bit = (op >> 3) & 0x07;
            core.registers.set_z(val & (1 << bit) == 0);
            core.registers.set_n(false);
            core.registers.set_h_flag(true);
        }
        2 => {
            let bit = (op >> 3) & 0x07;
            write_reg(core, idx, val & !(1 << bit));
        }
        _ => {
            let bit = (op >> 3) & 0x07;
            write_reg(core, idx, val | (1 << bit));
        }
    }
}

fn cb_compute(val: u8, op: u8, core: &mut Lr35902Cpu) -> u8 {
    match op >> 6 {
        0 => {
            let (r, c) = cb_rotate(val, op, core);
            set_rotate_flags(core, r, c);
            r
        }
        1 => {
            let bit = (op >> 3) & 0x07;
            core.registers.set_z(val & (1 << bit) == 0);
            core.registers.set_n(false);
            core.registers.set_h_flag(true);
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
