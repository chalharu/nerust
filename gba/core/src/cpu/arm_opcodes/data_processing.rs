use crate::cpu::arm_opcodes::helpers::barrel_shift;
use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let i = (instr >> 25) & 1 != 0;
    let opcode = ((instr >> 21) & 0xF) as u8;
    let s = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let rn_val = regs.r(rn);

    // Operand2
    let (op2, shifter_carry) = if i {
        let imm = (instr & 0xFF) as u32;
        let rot = ((instr >> 8) & 0xF) * 2;
        if rot == 0 {
            (imm, regs.cpsr_c())
        } else {
            let carry = (imm >> (rot - 1)) & 1 != 0;
            (imm.rotate_right(rot), carry)
        }
    } else {
        let rm = (instr & 0xF) as usize;
        let rm_val = regs.r(rm);
        let shift_type = ((instr >> 5) & 0b11) as u8;
        let shift_by_reg = (instr >> 4) & 1 != 0;
        if shift_by_reg {
            let rs = ((instr >> 8) & 0xF) as usize;
            let rs_val = regs.r(rs) & 0xFF;
            // Register shift takes +1 I cycle
            barrel_shift(rm_val, shift_type, rs_val, regs.cpsr_c())
        } else {
            let amount = ((instr >> 7) & 0x1F) as u32;
            barrel_shift(rm_val, shift_type, amount, regs.cpsr_c())
        }
    };

    let (result, carry, overflow) = match opcode {
        0x0 => {
            // AND
            let r = rn_val & op2;
            (r, shifter_carry, false)
        }
        0x1 => {
            // EOR
            let r = rn_val ^ op2;
            (r, shifter_carry, false)
        }
        0x2 => {
            // SUB
            let (r, c, v) = sub_with_flags(rn_val, op2);
            (r, c, v)
        }
        0x3 => {
            // RSB
            let (r, c, v) = sub_with_flags(op2, rn_val);
            (r, c, v)
        }
        0x4 => {
            // ADD
            let (r, c, v) = add_with_flags(rn_val, op2);
            (r, c, v)
        }
        0x5 => {
            // ADC
            let c_in = regs.cpsr_c() as u32;
            let (r, c, v) = adc_with_flags(rn_val, op2, c_in);
            (r, c, v)
        }
        0x6 => {
            // SBC
            let c_in = regs.cpsr_c() as u32;
            let (r, c, v) = sbc_with_flags(rn_val, op2, c_in);
            (r, c, v)
        }
        0x7 => {
            // RSC
            let c_in = regs.cpsr_c() as u32;
            let (r, c, v) = sbc_with_flags(op2, rn_val, c_in);
            (r, c, v)
        }
        0x8 => {
            // TST
            let r = rn_val & op2;
            (r, shifter_carry, false)
        }
        0x9 => {
            // TEQ
            let r = rn_val ^ op2;
            (r, shifter_carry, false)
        }
        0xA => {
            // CMP
            let (r, c, v) = sub_with_flags(rn_val, op2);
            (r, c, v)
        }
        0xB => {
            // CMN
            let (r, c, v) = add_with_flags(rn_val, op2);
            (r, c, v)
        }
        0xC => {
            // ORR
            let r = rn_val | op2;
            (r, shifter_carry, false)
        }
        0xD => {
            // MOV
            (op2, shifter_carry, false)
        }
        0xE => {
            // BIC
            let r = rn_val & !op2;
            (r, shifter_carry, false)
        }
        0xF => {
            // MVN
            let r = !op2;
            (r, shifter_carry, false)
        }
        _ => unreachable!(),
    };

    // TST/TEQ/CMP/CMN are flag-only, no Rd write
    let is_flag_only = matches!(opcode, 0x8..=0xB);
    if !is_flag_only {
        regs.set_r(rd, result);
        if rd == 15 && s {
            // MOVS PC,LR etc: CPSR = SPSR
            let spsr = regs.spsr();
            regs.set_cpsr(spsr);
        }
    }

    // Flag update
    if s {
        if is_flag_only || !matches!(opcode, 0x0 | 0x1 | 0xC | 0xD | 0xE | 0xF) {
            // Logical ops with S use shifter carry, arithmetic already computed
            regs.set_cpsr_n((result >> 31) & 1 != 0);
            regs.set_cpsr_z(result == 0);
            regs.set_cpsr_c(carry);
            regs.set_cpsr_v(overflow);
        } else {
            // Logical: N/Z from result, C from shifter, V unchanged
            regs.set_cpsr_n((result >> 31) & 1 != 0);
            regs.set_cpsr_z(result == 0);
            regs.set_cpsr_c(carry);
        }
        // Rd==15 with S already handled CPSR restore
    }

    // Cycle: +1 I if register shift
    let extra = if !i && ((instr >> 4) & 1) != 0 { 1 } else { 0 };
    1 + extra
}

fn add_with_flags(a: u32, b: u32) -> (u32, bool, bool) {
    let (r, c) = a.overflowing_add(b);
    let v = ((a ^ r) & (b ^ r) & 0x8000_0000) != 0;
    (r, c, v)
}

fn sub_with_flags(a: u32, b: u32) -> (u32, bool, bool) {
    let (r, c) = a.overflowing_sub(b);
    let v = ((a ^ b) & (a ^ r) & 0x8000_0000) != 0;
    // Carry is NOT borrow
    (r, !c, v) // overflowing_sub returns borrow as carry; invert
}

fn adc_with_flags(a: u32, b: u32, c_in: u32) -> (u32, bool, bool) {
    let (r1, c1) = a.overflowing_add(b);
    let (r, c2) = r1.overflowing_add(c_in);
    let c = c1 || c2;
    let v = ((a ^ r) & (b ^ r) & 0x8000_0000) != 0; // simplified
    (r, c, v)
}

fn sbc_with_flags(a: u32, b: u32, c_in: u32) -> (u32, bool, bool) {
    // SBC = A - B - !C
    let not_c = 1 - c_in;
    let (r1, c1) = a.overflowing_sub(b);
    let (r, c2) = r1.overflowing_sub(not_c);
    let c = !(c1 || c2);
    let v = ((a ^ b) & (a ^ r) & 0x8000_0000) != 0;
    (r, c, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::registers::CpuRegisters;
    use crate::memory::GbaMemoryBus;

    #[test]
    fn add_with_carry() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        // ADDS R0, R0, #1  with R0=0xFFFFFFFF should wrap to 0 with C=1
        regs.set_r(0, 0xFFFFFFFF);
        // ADDS R0, R0, #1 -> opcode 0xE2900001 (ADDS R0,R0,#1, S=1)
        let instr = 0xE2900001u32;
        handle(&mut regs, &mut bus, instr);
        assert_eq!(regs.r(0), 0);
        assert!(regs.cpsr_c());
    }

    #[test]
    fn mov_immediate() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        // MOV R0, #0xFF -> E3A000FF
        let instr = 0xE3A000FFu32;
        handle(&mut regs, &mut bus, instr);
        assert_eq!(regs.r(0), 0xFF);
    }
}
