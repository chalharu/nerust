use crate::cpu::registers::CpuRegisters;

pub fn handle(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let i = (instr >> 10) & 1 != 0;
    let op = (instr >> 9) & 1 != 0; // 0=ADD, 1=SUB
    let rn_field = ((instr >> 6) & 0x7) as usize;
    let rs = ((instr >> 3) & 0x7) as usize;
    let rd = (instr & 0x7) as usize;

    let rn_val = if i {
        (rn_field & 0x7) as u32
    } else {
        regs.r(rn_field)
    };
    let rs_val = regs.r(rs);
    let result = if op {
        let (r, _) = rs_val.overflowing_sub(rn_val);
        regs.set_cpsr_n((r >> 31) & 1 != 0);
        regs.set_cpsr_z(r == 0);
        regs.set_cpsr_c(rs_val >= rn_val);
        regs.set_cpsr_v(((rs_val ^ rn_val) & (rs_val ^ r) & 0x80000000) != 0);
        r
    } else {
        let (r, c) = rs_val.overflowing_add(rn_val);
        regs.set_cpsr_n((r >> 31) & 1 != 0);
        regs.set_cpsr_z(r == 0);
        regs.set_cpsr_c(c);
        regs.set_cpsr_v(((rs_val ^ r) & (rn_val ^ r) & 0x80000000) != 0);
        r
    };
    regs.set_r(rd, result);
    1
}
