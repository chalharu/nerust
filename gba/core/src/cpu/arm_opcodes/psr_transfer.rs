use crate::cpu_registers::CpuRegisters;

pub fn handle(regs: &mut CpuRegisters, instr: u32) -> u32 {
    let psr = (instr >> 22) & 1 != 0; // 0=CPSR, 1=SPSR
    let is_mrs = (instr >> 21) & 1 == 0 && (instr & 0x0FBF0FFF) == 0x010F0000;
    if is_mrs {
        read_psr(regs, instr, psr);
    } else {
        write_psr(regs, instr, psr);
    }
    1
}

fn read_psr(regs: &mut CpuRegisters, instr: u32, saved: bool) {
    let value = if saved { regs.spsr() } else { regs.cpsr() };
    regs.set_r(((instr >> 12) & 0xF) as usize, value);
}

fn write_psr(regs: &mut CpuRegisters, instr: u32, saved: bool) {
    let operand = psr_operand(regs, instr);
    let mut field_mask = (instr >> 16) & 0xF;
    if regs.cpsr_mode() == 0x10 {
        field_mask &= 0x8;
    }
    let current = if saved { regs.spsr() } else { regs.cpsr() };
    let value = apply_fields(current, operand, field_mask);
    if saved {
        regs.set_spsr(value);
    } else {
        regs.set_cpsr(value);
    }
}

fn psr_operand(regs: &CpuRegisters, instr: u32) -> u32 {
    if (instr >> 25) & 1 == 0 {
        return regs.r((instr & 0xF) as usize);
    }
    (instr & 0xFF).rotate_right(((instr >> 8) & 0xF) * 2)
}

fn apply_fields(mut current: u32, operand: u32, mask: u32) -> u32 {
    // ARM7TDMI defines control and NZCV fields here; reserved x/s bits stay unchanged.
    if mask & 1 != 0 {
        current = (current & 0xFFFFFF00) | (operand & 0xFF);
    }
    if mask & 8 != 0 {
        current = (current & 0x0FFFFFFF) | (operand & 0xF0000000);
    }
    current
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
