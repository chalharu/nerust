use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::cpu_opcodes::helpers::reg;
use crate::memory::GbcMemoryBus;

// ── ALU A, r8 (1-2 M-cycles) ───────────────────────────────

pub(crate) struct AluAR8;
impl CpuStepState for AluAR8 {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        let src = core.opcode & 0x07;
        let alu_op = (core.opcode >> 3) & 0x07;
        if src == 6 {
            if step == 0 { return StepResult::Continue; }
            alu_exec(core, alu_op, bus.read(core.registers.hl()));
            StepResult::Exit
        } else {
            let v = match src {
                0 => core.registers.b, 1 => core.registers.c,
                2 => core.registers.d, 3 => core.registers.e,
                4 => core.registers.h, 5 => core.registers.l,
                7 => core.registers.a, _ => 0,
            };
            alu_exec(core, alu_op, v);
            StepResult::Exit
        }
    }
}

// ── ALU A, d8 (2 M-cycles) ─────────────────────────────────

pub(crate) struct AluAD8<const OP: u8>;
impl<const OP: u8> CpuStepState for AluAD8<OP> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 { return StepResult::Continue; }
        let v = core.pc_read(bus);
        alu_exec(core, OP, v);
        StepResult::Exit
    }
}

fn alu_exec(core: &mut Lr35902Cpu, op: u8, v: u8) {
    let a = core.registers.a;
    match op {
        0 => {
            let r = a.wrapping_add(v);
            core.registers.set_h((a & 0x0F) + (v & 0x0F) > 0x0F);
            core.registers.set_c((a as u16) + (v as u16) > 0xFF);
            core.registers.set_z(r == 0); core.registers.set_n(false); core.registers.a = r;
        }
        1 => {
            let c = core.registers.c_flag() as u8;
            let r = a.wrapping_add(v).wrapping_add(c);
            core.registers.set_h((a & 0x0F) + (v & 0x0F) + c > 0x0F);
            core.registers.set_c((a as u16) + (v as u16) + (c as u16) > 0xFF);
            core.registers.set_z(r == 0); core.registers.set_n(false); core.registers.a = r;
        }
        2 => {
            core.registers.set_h((a & 0x0F) < (v & 0x0F));
            core.registers.set_c(a < v);
            core.registers.a = a.wrapping_sub(v);
            core.registers.set_z(core.registers.a == 0); core.registers.set_n(true);
        }
        3 => {
            let c_flag = core.registers.c_flag();
            let c = c_flag as u16;
            let lower_sum = (v & 0x0F) as u16 + c;
            let total = (v as u16) + c;
            core.registers.set_h(((a & 0x0F) as u16) < lower_sum);
            core.registers.set_c((a as u16) < total);
            core.registers.a = a.wrapping_sub(v).wrapping_sub(c_flag as u8);
            core.registers.set_z(core.registers.a == 0); core.registers.set_n(true);
        }
        4 => { core.registers.a &= v; core.registers.set_z(core.registers.a == 0); core.registers.set_n(false); core.registers.set_h(true); core.registers.set_c(false); }
        5 => { core.registers.a ^= v; core.registers.set_z(core.registers.a == 0); core.registers.set_n(false); core.registers.set_h(false); core.registers.set_c(false); }
        6 => { core.registers.a |= v; core.registers.set_z(core.registers.a == 0); core.registers.set_n(false); core.registers.set_h(false); core.registers.set_c(false); }
        7 => { core.registers.set_h((a & 0x0F) < (v & 0x0F)); core.registers.set_c(a < v); core.registers.set_z(a.wrapping_sub(v) == 0); core.registers.set_n(true); }
        _ => {}
    }
}

// ── ADD HL, r16 (2 M-cycles) ───────────────────────────────

pub(crate) struct AddHlR16<const R: u8>;
impl<const R: u8> CpuStepState for AddHlR16<R> {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 { return StepResult::Continue; }
        let hl = core.registers.hl();
        let v = match R {
            reg::BC => core.registers.bc(), reg::DE => core.registers.de(),
            reg::R16_HL => core.registers.hl(), _ => core.registers.sp,
        };
        core.registers.set_h((hl & 0x0FFF) + (v & 0x0FFF) > 0x0FFF);
        core.registers.set_c((hl as u32) + (v as u32) > 0xFFFF);
        core.registers.set_n(false);
        core.registers.set_hl(hl.wrapping_add(v));
        StepResult::Exit
    }
}

pub(crate) struct AddHlSp;
impl CpuStepState for AddHlSp {
    fn exec(core: &mut Lr35902Cpu, _: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 { return StepResult::Continue; }
        let hl = core.registers.hl();
        let sp = core.registers.sp;
        core.registers.set_h((hl & 0x0FFF) + (sp & 0x0FFF) > 0x0FFF);
        core.registers.set_c((hl as u32) + (sp as u32) > 0xFFFF);
        core.registers.set_n(false);
        core.registers.set_hl(hl.wrapping_add(sp));
        StepResult::Exit
    }
}

// ── ADD SP, e (4 M-cycles) ─────────────────────────────────

pub(crate) struct AddSpE;
impl CpuStepState for AddSpE {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 { return StepResult::Continue; }
        if step == 1 { core.operands[0] = core.pc_read(bus); return StepResult::Continue; }
        if step == 2 { return StepResult::Continue; }
        if step == 3 {
            let offset = core.operands[0] as i8;
            let sp = core.registers.sp;
            let r = sp.wrapping_add_signed(offset as i16);
            core.registers.set_h((sp & 0x000F) + (offset as u8 as u16 & 0x000F) > 0x000F);
            core.registers.set_c((sp & 0x00FF) + (offset as u8 as u16 & 0x00FF) > 0x00FF);
            core.registers.set_z(false); core.registers.set_n(false); core.registers.sp = r;
            return StepResult::Exit;
        }
        unreachable!()
    }
}
