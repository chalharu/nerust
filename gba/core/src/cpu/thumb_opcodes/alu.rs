use crate::cpu_registers::CpuRegisters;

pub fn handle(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let op = ((instr >> 6) & 0xF) as u8;
    let rs = ((instr >> 3) & 0x7) as usize;
    let rd = (instr & 0x7) as usize;
    let rs_val = regs.r(rs);
    let rd_val = regs.r(rd);
    match op {
        0b0000 => {
            // AND
            let r = rd_val & rs_val;
            regs.set_r(rd, r);
            update_nz(regs, r);
            1
        }
        0b0001 => {
            // EOR
            let r = rd_val ^ rs_val;
            regs.set_r(rd, r);
            update_nz(regs, r);
            1
        }
        0b0010 => {
            // LSL Rd, Rs
            let shift = rs_val & 0xFF;
            let (r, c) = if shift == 0 {
                (rd_val, regs.cpsr_c())
            } else if shift < 32 {
                let c = (rd_val >> (32 - shift)) & 1 != 0;
                (rd_val << shift, c)
            } else if shift == 32 {
                let c = rd_val & 1 != 0;
                (0, c)
            } else {
                (0, false)
            };
            regs.set_r(rd, r);
            update_nz(regs, r);
            regs.set_cpsr_c(c);
            1 + u32::from(shift != 0)
        }
        0b0011 => {
            // LSR Rd, Rs
            let shift = rs_val & 0xFF;
            let (r, c) = if shift == 0 {
                (rd_val, regs.cpsr_c())
            } else if shift < 32 {
                let c = (rd_val >> (shift - 1)) & 1 != 0;
                (rd_val >> shift, c)
            } else if shift == 32 {
                let c = (rd_val >> 31) & 1 != 0;
                (0, c)
            } else {
                (0, false)
            };
            regs.set_r(rd, r);
            update_nz(regs, r);
            regs.set_cpsr_c(c);
            2
        }
        0b0100 => {
            // ASR Rd, Rs
            let shift = rs_val & 0xFF;
            let (r, c) = if shift == 0 {
                (rd_val, regs.cpsr_c())
            } else if shift < 32 {
                let c = (rd_val >> (shift - 1)) & 1 != 0;
                let v = ((rd_val as i32) >> shift) as u32;
                (v, c)
            } else {
                let c = (rd_val >> 31) & 1 != 0;
                let v = if c { 0xFFFFFFFF } else { 0 };
                (v, c)
            };
            regs.set_r(rd, r);
            update_nz(regs, r);
            regs.set_cpsr_c(c);
            2
        }
        0b0101 => {
            // ADC Rd, Rs
            let c_in = regs.cpsr_c() as u32;
            let (r1, c1) = rd_val.overflowing_add(rs_val);
            let (r, c2) = r1.overflowing_add(c_in);
            let c = c1 || c2;
            regs.set_r(rd, r);
            update_nz(regs, r);
            regs.set_cpsr_c(c);
            regs.set_cpsr_v(((rd_val ^ r) & (rs_val ^ r) & 0x80000000) != 0);
            1
        }
        0b0110 => {
            // SBC Rd, Rs
            let c_in = regs.cpsr_c() as u32;
            let not_c = 1 - c_in;
            let (r1, c1) = rd_val.overflowing_sub(rs_val);
            let (r, c2) = r1.overflowing_sub(not_c);
            let c = !(c1 || c2);
            regs.set_r(rd, r);
            update_nz(regs, r);
            regs.set_cpsr_c(c);
            regs.set_cpsr_v(((rd_val ^ rs_val) & (rd_val ^ r) & 0x80000000) != 0);
            1
        }
        0b0111 => {
            // ROR Rd, Rs
            let shift = rs_val & 0xFF;
            let (r, c) = if shift == 0 {
                (rd_val, regs.cpsr_c())
            } else if shift.is_multiple_of(32) {
                let c = (rd_val >> 31) & 1 != 0;
                (rd_val, c)
            } else {
                let rot = shift % 32;
                let c = (rd_val >> (rot - 1)) & 1 != 0;
                (rd_val.rotate_right(rot), c)
            };
            regs.set_r(rd, r);
            update_nz(regs, r);
            regs.set_cpsr_c(c);
            2
        }
        0b1000 => {
            // TST
            let r = rd_val & rs_val;
            update_nz(regs, r);
            // C from barrel? For TST, shifter not involved, keep C
            1
        }
        0b1001 => {
            // NEG Rd, Rs => Rd = 0 - Rs
            let (r, _) = (0u32).overflowing_sub(rs_val);
            regs.set_r(rd, r);
            update_nz(regs, r);
            regs.set_cpsr_c(rs_val == 0);
            regs.set_cpsr_v(rs_val == 0x80000000);
            1
        }
        0b1010 => {
            // CMP Rd, Rs
            let (r, _) = rd_val.overflowing_sub(rs_val);
            update_nz(regs, r);
            regs.set_cpsr_c(rd_val >= rs_val);
            regs.set_cpsr_v(((rd_val ^ rs_val) & (rd_val ^ r) & 0x80000000) != 0);
            1
        }
        0b1011 => {
            // CMN Rd, Rs
            let (r, c) = rd_val.overflowing_add(rs_val);
            update_nz(regs, r);
            regs.set_cpsr_c(c);
            regs.set_cpsr_v(((rd_val ^ r) & (rs_val ^ r) & 0x80000000) != 0);
            1
        }
        0b1100 => {
            // ORR
            let r = rd_val | rs_val;
            regs.set_r(rd, r);
            update_nz(regs, r);
            1
        }
        0b1101 => {
            // MUL
            let r = rd_val.wrapping_mul(rs_val);
            regs.set_r(rd, r);
            update_nz(regs, r);
            3 // Thumb MUL takes 3-4 cycles
        }
        0b1110 => {
            // BIC
            let r = rd_val & !rs_val;
            regs.set_r(rd, r);
            update_nz(regs, r);
            1
        }
        0b1111 => {
            // MVN
            let r = !rs_val;
            regs.set_r(rd, r);
            update_nz(regs, r);
            1
        }
        _ => 1,
    }
}

fn update_nz(regs: &mut CpuRegisters, r: u32) {
    regs.set_cpsr_n((r >> 31) & 1 != 0);
    regs.set_cpsr_z(r == 0);
}

pub fn handle_imm(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let op = (instr >> 11) & 0b11;
    let rd = ((instr >> 8) & 0x7) as usize;
    let imm = (instr & 0xFF) as u32;
    match op {
        0b00 => {
            regs.set_r(rd, imm);
            regs.set_cpsr_n(false);
            regs.set_cpsr_z(imm == 0);
        }
        0b01 => {
            let rd_val = regs.r(rd);
            let (r, _) = rd_val.overflowing_sub(imm);
            regs.set_cpsr_n((r >> 31) & 1 != 0);
            regs.set_cpsr_z(r == 0);
            regs.set_cpsr_c(rd_val >= imm);
            regs.set_cpsr_v(((rd_val ^ imm) & (rd_val ^ r) & 0x80000000) != 0);
        }
        0b10 => {
            let rd_val = regs.r(rd);
            let (r, c) = rd_val.overflowing_add(imm);
            regs.set_r(rd, r);
            regs.set_cpsr_n((r >> 31) & 1 != 0);
            regs.set_cpsr_z(r == 0);
            regs.set_cpsr_c(c);
            regs.set_cpsr_v(((rd_val ^ r) & (imm ^ r) & 0x80000000) != 0);
        }
        0b11 => {
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
    let sp = (instr >> 11) & 1 != 0;
    let rd = ((instr >> 8) & 0x7) as usize;
    let imm = ((instr & 0xFF) as u32) << 2;
    let base = if sp { regs.sp() } else { regs.pc() & !3 };
    regs.set_r(rd, base.wrapping_add(imm));
    1
}

pub fn handle_sp_offset(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let s = (instr >> 7) & 1 != 0;
    let imm = ((instr & 0x7F) as u32) << 2;
    if s {
        regs.set_sp(regs.sp().wrapping_sub(imm));
    } else {
        regs.set_sp(regs.sp().wrapping_add(imm));
    }
    1
}
