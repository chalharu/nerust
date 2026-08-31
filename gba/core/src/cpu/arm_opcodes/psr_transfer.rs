use crate::cpu::registers::CpuRegisters;

pub fn handle(regs: &mut CpuRegisters, instr: u32) -> u32 {
    let psr = (instr >> 22) & 1 != 0; // 0=CPSR, 1=SPSR
    let is_mrs = (instr >> 21) & 1 == 0 && (instr & 0x0FBF0FFF) == 0x010F0000;
    if is_mrs {
        // MRS Rd, psr
        let rd = ((instr >> 12) & 0xF) as usize;
        let val = if psr { regs.spsr() } else { regs.cpsr() };
        regs.set_r(rd, val);
    } else {
        let operand = if (instr >> 25) & 1 != 0 {
            let imm = instr & 0xFF;
            imm.rotate_right(((instr >> 8) & 0xF) * 2)
        } else {
            regs.r((instr & 0xF) as usize)
        };
        let mut field_mask = (instr >> 16) & 0xF;
        let privileged = regs.cpsr_mode() != 0x10;
        if !privileged {
            field_mask &= 0x8; // USRは条件フラグのみ変更可能
        }
        let mut psr_val = if psr { regs.spsr() } else { regs.cpsr() };
        // field_mask: bit0=c, bit1=x, bit2=s, bit3=f。
        // ARM7TDMIで定義されるCPSR/SPSRビットだけを書き換え、予約ビットは保持する。
        if field_mask & 1 != 0 {
            psr_val = (psr_val & 0xFFFFFF00) | (operand & 0x000000FF);
        }
        if field_mask & 8 != 0 {
            psr_val = (psr_val & 0x0FFFFFFF) | (operand & 0xF0000000);
        }
        if psr {
            regs.set_spsr(psr_val);
        } else {
            regs.set_cpsr(psr_val);
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msr_updates_flags_field() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(0, 0xFF000000);
        handle(&mut regs, 0xE128F000); // MSR CPSR_f, R0
        assert_eq!(regs.cpsr() & 0xF0000000, 0xF0000000);
        assert_eq!(regs.cpsr() & 0x0F000000, 0);
        assert_eq!(regs.cpsr_mode(), 0x1F);
    }

    #[test]
    fn user_msr_cannot_change_control_field() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_cpsr(0x10);
        regs.set_r(0, 0x13);
        handle(&mut regs, 0xE121F000); // MSR CPSR_c, R0
        assert_eq!(regs.cpsr_mode(), 0x10);
    }
}
