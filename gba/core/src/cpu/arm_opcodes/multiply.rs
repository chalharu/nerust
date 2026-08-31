use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let is_long = (instr >> 23) & 1 != 0;
    if is_long {
        // Multiply Long: UMULL/UMLAL/SMULL/SMLAL
        let signed = (instr >> 22) & 1 != 0;
        let a = (instr >> 21) & 1 != 0;
        let s = (instr >> 20) & 1 != 0;
        let rd_hi = ((instr >> 16) & 0xF) as usize;
        let rd_lo = ((instr >> 12) & 0xF) as usize;
        let rs = ((instr >> 8) & 0xF) as usize;
        let rm = (instr & 0xF) as usize;

        let rs_val = regs.r(rs);
        let rm_val = regs.r(rm);
        let (hi, lo) = if !signed {
            let res = (rm_val as u64).wrapping_mul(rs_val as u64);
            if a {
                let lo_val = regs.r(rd_lo) as u64;
                let hi_val = regs.r(rd_hi) as u64;
                let acc = (hi_val << 32) | lo_val;
                let res_acc = res.wrapping_add(acc);
                ((res_acc >> 32) as u32, res_acc as u32)
            } else {
                ((res >> 32) as u32, res as u32)
            }
        } else {
            let res = (rm_val as i32 as i64).wrapping_mul(rs_val as i32 as i64) as u64;
            if a {
                let lo_val = regs.r(rd_lo) as u64;
                let hi_val = regs.r(rd_hi) as u64;
                let acc = (hi_val << 32) | lo_val;
                let res_acc = res.wrapping_add(acc);
                ((res_acc >> 32) as u32, res_acc as u32)
            } else {
                ((res >> 32) as u32, res as u32)
            }
        };
        regs.set_r(rd_hi, hi);
        regs.set_r(rd_lo, lo);
        if s {
            regs.set_cpsr_n((hi >> 31) & 1 != 0);
            regs.set_cpsr_z(hi == 0 && lo == 0);
        }
        return multiplier_cycles(rs_val) + 1 + u32::from(a);
    }

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
    }

    let cycles = multiplier_cycles(rs_val);
    if a { cycles + 1 } else { cycles }
}

fn multiplier_cycles(rs_val: u32) -> u32 {
    if rs_val & 0xFFFFFF00 == 0 || rs_val & 0xFFFFFF00 == 0xFFFFFF00 {
        1
    } else if rs_val & 0xFFFF0000 == 0 || rs_val & 0xFFFF0000 == 0xFFFF0000 {
        2
    } else if rs_val & 0xFF000000 == 0 || rs_val & 0xFF000000 == 0xFF000000 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu_registers::CpuRegisters;
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
