use crate::cpu::arm_opcodes::helpers::{barrel_shift, barrel_shift_register};
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let i = (instr >> 25) & 1 != 0;
    let opcode = ((instr >> 21) & 0xF) as u8;
    let s = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let (op2, shifter_carry) = operand2(regs, instr, i);
    let (result, carry, overflow) = execute(opcode, regs.r(rn), op2, shifter_carry, regs.cpsr_c());
    let flag_only = matches!(opcode, 0x8..=0xB);
    if !flag_only {
        write_result(regs, rd, result, s);
    }
    if s && !(rd == 15 && !flag_only) {
        update_flags(regs, opcode, result, carry, overflow);
    }
    1 + u32::from(!i && ((instr >> 4) & 1) != 0)
}

fn operand2(regs: &CpuRegisters, instr: u32, immediate: bool) -> (u32, bool) {
    if immediate {
        let imm = instr & 0xFF;
        let rot = ((instr >> 8) & 0xF) * 2;
        return immediate_operand(imm, rot, regs.cpsr_c());
    }
    let value = regs.r((instr & 0xF) as usize);
    let shift_type = ((instr >> 5) & 0b11) as u8;
    if (instr >> 4) & 1 != 0 {
        let amount = regs.r(((instr >> 8) & 0xF) as usize) & 0xFF;
        barrel_shift_register(value, shift_type, amount, regs.cpsr_c())
    } else {
        barrel_shift(value, shift_type, (instr >> 7) & 0x1F, regs.cpsr_c())
    }
}

fn immediate_operand(value: u32, rotation: u32, carry: bool) -> (u32, bool) {
    if rotation == 0 {
        (value, carry)
    } else {
        (
            value.rotate_right(rotation),
            value & (1 << (rotation - 1)) != 0,
        )
    }
}

fn execute(opcode: u8, left: u32, right: u32, shift_carry: bool, carry: bool) -> (u32, bool, bool) {
    match opcode {
        0x0 => (left & right, shift_carry, false),
        0x1 => (left ^ right, shift_carry, false),
        0x2 | 0xA => sub_with_flags(left, right),
        0x3 => sub_with_flags(right, left),
        0x4 | 0xB => add_with_flags(left, right),
        0x5 => adc_with_flags(left, right, u32::from(carry)),
        0x6 => sbc_with_flags(left, right, u32::from(carry)),
        0x7 => sbc_with_flags(right, left, u32::from(carry)),
        0x8 => (left & right, shift_carry, false),
        0x9 => (left ^ right, shift_carry, false),
        0xC => (left | right, shift_carry, false),
        0xD => (right, shift_carry, false),
        0xE => (left & !right, shift_carry, false),
        0xF => (!right, shift_carry, false),
        _ => unreachable!(),
    }
}

fn write_result(regs: &mut CpuRegisters, destination: usize, result: u32, set_flags: bool) {
    regs.set_r(destination, result);
    if destination == 15 && set_flags {
        regs.set_cpsr(regs.spsr());
    }
}

fn update_flags(regs: &mut CpuRegisters, opcode: u8, result: u32, carry: bool, overflow: bool) {
    crate::cpu::arm_opcodes::helpers::update_nz(regs, result);
    regs.set_cpsr_c(carry);
    if !matches!(opcode, 0x0 | 0x1 | 0x8 | 0x9 | 0xC..=0xF) {
        regs.set_cpsr_v(overflow);
    }
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
    let signed = a as i32 as i64 + b as i32 as i64 + i64::from(c_in);
    let v = signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN);
    (r, c, v)
}

fn sbc_with_flags(a: u32, b: u32, c_in: u32) -> (u32, bool, bool) {
    // SBC = A - B - !C
    let not_c = 1 - c_in;
    let (r1, c1) = a.overflowing_sub(b);
    let (r, c2) = r1.overflowing_sub(not_c);
    let c = !(c1 || c2);
    let signed = a as i32 as i64 - b as i32 as i64 - i64::from(not_c);
    let v = signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN);
    (r, c, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu_registers::CpuRegisters;
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

    #[test]
    fn tst_preserves_overflow() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_cpsr_v(true);
        regs.set_r(0, 1);
        let mut bus = GbaMemoryBus::new();
        handle(&mut regs, &mut bus, 0xE3100001); // TST R0,#1
        assert!(regs.cpsr_v());
    }
}
