use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let a = (instr >> 21) & 1 != 0; // MLA if 1
    let s = (instr >> 20) & 1 != 0;
    let rd = ((instr >> 16) & 0xF) as usize;
    let rn = ((instr >> 12) & 0xF) as usize;
    let rs = ((instr >> 8) & 0xF) as usize;
    let rm = (instr & 0xF) as usize;

    let rs_val = regs.r(rs);
    let rm_val = regs.r(rm);
    let mut result = rm_val.wrapping_mul(rs_val);
    if a {
        result = result.wrapping_add(regs.r(rn));
    }
    regs.set_r(rd, result);

    if s {
        regs.set_cpsr_n((result >> 31) & 1 != 0);
        regs.set_cpsr_z(result == 0);
        // C and V unchanged for MUL?
    }

    // Multiply cycles: based on Rs value (early termination)
    let cycles = if rs_val & 0xFFFFFF00 == 0 || rs_val & 0xFFFFFF00 == 0xFFFFFF00 {
        1
    } else if rs_val & 0xFFFF0000 == 0 || rs_val & 0xFFFF0000 == 0xFFFF0000 {
        2
    } else if rs_val & 0xFF000000 == 0 || rs_val & 0xFF000000 == 0xFF000000 {
        3
    } else {
        4
    };
    if a { cycles + 1 } else { cycles }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::registers::CpuRegisters;
    use crate::memory::GbaMemoryBus;

    #[test]
    fn mul_simple() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        regs.set_r(1, 3);
        regs.set_r(2, 4);
        // MUL R0, R1, R2 -> E0000290? Actually MUL R0,R1,R2 = E0000192?
        // Encoding: 0xE0000291? Let's use MUL R0,R1,R2 = E0000091 with Rs=1, Rm=2
        // 0xE0000091: cond E, 000,000,0,0,0, Rd=0, Rn=0, Rs=1, 1001, Rm=2
        // Simplified: Use our handler directly
        regs.set_r(0, 0);
        let instr = 0xE0000291u32; // MUL R0, R2, R1 (Rd=0, Rs=2, Rm=1)
        handle(&mut regs, &mut bus, instr);
        assert_eq!(regs.r(0), 12);
    }
}
