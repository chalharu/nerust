use crate::cpu_registers::CpuRegisters;

pub fn handle(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let op = ((instr >> 6) & 0xF) as u8;
    let rs = ((instr >> 3) & 0x7) as usize;
    let rd = (instr & 0x7) as usize;
    let rs_val = regs.r(rs);
    let rd_val = regs.r(rd);
    // Thumb ALU opcodes: AND/EOR, shifts, ADC/SBC, ROR, TST,
    // NEG/CMP/CMN, ORR/MUL/BIC/MVN.
    match op {
        0x0 | 0x1 | 0xC..=0xF => logical(regs, rd, op, rd_val, rs_val),
        0x2..=0x4 | 0x7 => shift(regs, rd, op, rd_val, rs_val),
        0x5 | 0x6 => carry_arithmetic(regs, rd, op, rd_val, rs_val),
        0x8 => test(regs, rd_val, rs_val),
        0x9..=0xB => compare(regs, rd, op, rd_val, rs_val),
        _ => 1,
    }
}

fn logical(regs: &mut CpuRegisters, destination: usize, op: u8, left: u32, right: u32) -> u32 {
    let result = match op {
        0x0 => left & right,             // AND
        0x1 => left ^ right,             // EOR
        0xC => left | right,             // ORR
        0xD => left.wrapping_mul(right), // MUL
        0xE => left & !right,            // BIC
        _ => !right,                     // MVN
    };
    regs.set_r(destination, result);
    update_nz(regs, result);
    if op == 0xD { 3 } else { 1 }
}

fn shift(regs: &mut CpuRegisters, destination: usize, op: u8, value: u32, amount: u32) -> u32 {
    // Register shift amount zero preserves both the value and carry flag.
    let shift_type = match op {
        0x2 => 0, // LSL
        0x3 => 1, // LSR
        0x4 => 2, // ASR
        _ => 3,   // ROR
    };
    let (result, carry) = crate::cpu::arm_opcodes::helpers::barrel_shift_register(
        value,
        shift_type,
        amount,
        regs.cpsr_c(),
    );
    regs.set_r(destination, result);
    update_nz(regs, result);
    regs.set_cpsr_c(carry);
    if op == 0x2 && amount & 0xFF == 0 {
        1
    } else {
        2
    }
}

fn carry_arithmetic(
    regs: &mut CpuRegisters,
    destination: usize,
    op: u8,
    left: u32,
    right: u32,
) -> u32 {
    let carry_in = u32::from(regs.cpsr_c());
    let (result, carry, overflow) = if op == 0x5 {
        add_with_carry(left, right, carry_in)
    } else {
        subtract_with_carry(left, right, carry_in)
    };
    regs.set_r(destination, result);
    update_nz(regs, result);
    regs.set_cpsr_c(carry);
    regs.set_cpsr_v(overflow);
    1
}

fn add_with_carry(left: u32, right: u32, carry_in: u32) -> (u32, bool, bool) {
    let sum = u64::from(left) + u64::from(right) + u64::from(carry_in);
    let result = sum as u32;
    let overflow = ((left ^ result) & (right ^ result) & 0x80000000) != 0;
    (result, sum > u64::from(u32::MAX), overflow)
}

fn subtract_with_carry(left: u32, right: u32, carry_in: u32) -> (u32, bool, bool) {
    let borrow = 1 - carry_in;
    let result = left.wrapping_sub(right).wrapping_sub(borrow);
    let carry = u64::from(left) >= u64::from(right) + u64::from(borrow);
    let overflow = ((left ^ right) & (left ^ result) & 0x80000000) != 0;
    (result, carry, overflow)
}

fn test(regs: &mut CpuRegisters, left: u32, right: u32) -> u32 {
    // TST has no shifter operand in Thumb, so C is preserved.
    update_nz(regs, left & right);
    1
}

fn compare(regs: &mut CpuRegisters, destination: usize, op: u8, left: u32, right: u32) -> u32 {
    let (result, carry, overflow) = match op {
        0x9 => (0u32.wrapping_sub(right), right == 0, right == 0x80000000),
        0xA => {
            let result = left.wrapping_sub(right);
            (
                result,
                left >= right,
                ((left ^ right) & (left ^ result) & 0x80000000) != 0,
            )
        }
        _ => {
            let (result, carry) = left.overflowing_add(right);
            (
                result,
                carry,
                ((left ^ result) & (right ^ result) & 0x80000000) != 0,
            )
        }
    };
    if op == 0x9 {
        regs.set_r(destination, result);
    }
    update_nz(regs, result);
    regs.set_cpsr_c(carry);
    regs.set_cpsr_v(overflow);
    1
}

fn update_nz(regs: &mut CpuRegisters, r: u32) {
    crate::cpu::arm_opcodes::helpers::update_nz(regs, r);
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
            crate::cpu::arm_opcodes::helpers::update_nz(regs, r);
            regs.set_cpsr_c(rd_val >= imm);
            regs.set_cpsr_v(((rd_val ^ imm) & (rd_val ^ r) & 0x80000000) != 0);
        }
        0b10 => {
            let rd_val = regs.r(rd);
            let (r, c) = rd_val.overflowing_add(imm);
            regs.set_r(rd, r);
            crate::cpu::arm_opcodes::helpers::update_nz(regs, r);
            regs.set_cpsr_c(c);
            regs.set_cpsr_v(((rd_val ^ r) & (imm ^ r) & 0x80000000) != 0);
        }
        0b11 => {
            let rd_val = regs.r(rd);
            let (r, _) = rd_val.overflowing_sub(imm);
            regs.set_r(rd, r);
            crate::cpu::arm_opcodes::helpers::update_nz(regs, r);
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
