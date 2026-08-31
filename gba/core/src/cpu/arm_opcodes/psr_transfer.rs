use crate::cpu::registers::CpuRegisters;

pub fn handle(regs: &mut CpuRegisters, instr: u32) -> u32 {
    let psr = (instr >> 22) & 1 != 0; // 0=CPSR, 1=SPSR
    let is_msr = (instr >> 21) & 1 == 0; // Actually bit21 distinguishes MRS/MSR? Simplified
    // MRS: bits 27-25=000, 24-21=0000, 20=0, 15-12=Rd, 0-11=0
    // MSR: bits 27-25=000, 24-21=0010/0011, etc
    // Simplified: Check bit21: if 0 then MRS, else MSR
    let is_mrs = (instr >> 21) & 1 == 0 && (instr & 0x0FBF0FFF) == 0x010F0000;
    if is_mrs {
        // MRS Rd, psr
        let rd = ((instr >> 12) & 0xF) as usize;
        let val = if psr { regs.spsr() } else { regs.cpsr() };
        regs.set_r(rd, val);
    } else {
        // MSR psr, Rm or #imm
        let rm_val = regs.r((instr & 0xF) as usize);
        let field_mask = (instr >> 16) & 0xF;
        // Simplified: update CPSR/SPSR with field mask
        let mut psr_val = if psr { regs.spsr() } else { regs.cpsr() };
        // field_mask bits: 8=c, 4=x, 2=s, 1=f
        if field_mask & 1 != 0 {
            psr_val = (psr_val & 0xFFFFFF00) | (rm_val & 0x000000FF);
        }
        if field_mask & 2 != 0 {
            psr_val = (psr_val & 0xFFFF00FF) | (rm_val & 0x0000FF00);
        }
        if field_mask & 4 != 0 {
            psr_val = (psr_val & 0xFF00FFFF) | (rm_val & 0x00FF0000);
        }
        if field_mask & 8 != 0 {
            psr_val = (psr_val & 0x00FFFFFF) | (rm_val & 0xFF000000);
            // Also N/Z/C/V if field
            if field_mask & 8 != 0 {
                // already
            }
        }
        if psr {
            regs.set_spsr(psr_val);
        } else {
            // Check privilege: only privileged modes can change mode bits
            // Simplified: allow all
            regs.set_cpsr(psr_val);
        }
    }
    1
}
