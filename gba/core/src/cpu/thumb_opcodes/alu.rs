use crate::cpu::registers::CpuRegisters;

pub fn handle(_regs: &mut CpuRegisters, _instr: u16) -> u32 {
    // Format 4: ALU operations (010000)
    // Op 6-9, Rs 3-5, Rd 0-2
    1
}

pub fn handle_imm(regs: &mut CpuRegisters, instr: u16) -> u32 {
    // Format 3: 001 Op Rd Offset8
    let op = (instr >> 11) & 0b11;
    let rd = ((instr >> 8) & 0x7) as usize;
    let imm = (instr & 0xFF) as u32;
    match op {
        0b00 => {
            // MOV Rd, #imm
            regs.set_r(rd, imm);
            regs.set_cpsr_n(false);
            regs.set_cpsr_z(imm == 0);
        }
        0b01 => {
            // CMP Rd, #imm
            let rd_val = regs.r(rd);
            let (r, c) = rd_val.overflowing_sub(imm);
            regs.set_cpsr_n((r >> 31) & 1 != 0);
            regs.set_cpsr_z(r == 0);
            regs.set_cpsr_c(rd_val >= imm);
            regs.set_cpsr_v(((rd_val ^ imm) & (rd_val ^ r) & 0x80000000) != 0);
        }
        0b10 => {
            // ADD Rd, #imm
            let rd_val = regs.r(rd);
            let (r, c) = rd_val.overflowing_add(imm);
            regs.set_r(rd, r);
            regs.set_cpsr_n((r >> 31) & 1 != 0);
            regs.set_cpsr_z(r == 0);
            regs.set_cpsr_c(c);
            regs.set_cpsr_v(((rd_val ^ r) & (imm ^ r) & 0x80000000) != 0);
        }
        0b11 => {
            // SUB Rd, #imm
            let rd_val = regs.r(rd);
            let (r, _) = rd_val.overflowing_sub(imm);
            regs.set_r(rd, r);
            regs.set_cpsr_n((r >> 31) & 1 != 0);
            regs.set_cpsr_z(r == 0);
            regs.set_cpsr_c(rd_val >= imm);
            regs.set_cpsr_v(((rd_val ^ imm) & (rd_val ^ r) & 0x80000000) != 0);
        }
        _ => {}
    }
    1
}

pub fn handle_load_address(regs: &mut CpuRegisters, instr: u16) -> u32 {
    // Format 12: ADD Rd, PC/SP, #imm*4
    let sp = (instr >> 11) & 1 != 0;
    let rd = ((instr >> 8) & 0x7) as usize;
    let imm = ((instr & 0xFF) as u32) << 2;
    let base = if sp { regs.sp() } else { (regs.pc() + 2) & !2 };
    regs.set_r(rd, base.wrapping_add(imm));
    1
}

pub fn handle_sp_offset(regs: &mut CpuRegisters, instr: u16) -> u32 {
    // Format 13: ADD SP, #imm
    let s = (instr >> 7) & 1 != 0;
    let imm = ((instr & 0x7F) as u32) << 2;
    if s {
        regs.set_sp(regs.sp().wrapping_sub(imm));
    } else {
        regs.set_sp(regs.sp().wrapping_add(imm));
    }
    1
}
