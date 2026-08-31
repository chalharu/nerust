use crate::cpu_registers::CpuRegisters;

pub fn handle(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let op = (instr >> 11) & 0b11;
    let offset = ((instr >> 6) & 0x1F) as u32;
    let rs = ((instr >> 3) & 0x7) as usize;
    let rd = (instr & 0x7) as usize;
    let rs_val = regs.r(rs);
    let (result, carry) = match op {
        0b00 => {
            // LSL
            if offset == 0 {
                (rs_val, regs.cpsr_c())
            } else {
                let c = (rs_val >> (32 - offset)) & 1 != 0;
                (rs_val << offset, c)
            }
        }
        0b01 => {
            // LSR
            if offset == 0 {
                // LSR #32
                let c = (rs_val >> 31) & 1 != 0;
                (0, c)
            } else {
                let c = (rs_val >> (offset - 1)) & 1 != 0;
                (rs_val >> offset, c)
            }
        }
        0b10 => {
            // ASR
            if offset == 0 {
                let c = (rs_val >> 31) & 1 != 0;
                let v = if c { 0xFFFFFFFF } else { 0 };
                (v, c)
            } else {
                let c = (rs_val >> (offset - 1)) & 1 != 0;
                let v = ((rs_val as i32) >> offset) as u32;
                (v, c)
            }
        }
        _ => (0, false),
    };
    regs.set_r(rd, result);
    regs.set_cpsr_n((result >> 31) & 1 != 0);
    regs.set_cpsr_z(result == 0);
    regs.set_cpsr_c(carry);
    1
}
