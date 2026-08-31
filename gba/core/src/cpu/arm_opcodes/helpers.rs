/// Barrel shifter: LSL/LSR/ASR/ROR + RRX
/// shift_type: 0=LSL, 1=LSR, 2=ASR, 3=ROR
pub fn barrel_shift(rm: u32, shift_type: u8, amount: u32, carry_in: bool) -> (u32, bool) {
    let amount = amount & 0xFF;
    match shift_type & 0b11 {
        0b00 => shift_lsl(rm, amount, carry_in),
        0b01 => shift_lsr(rm, amount),
        0b10 => shift_asr(rm, amount),
        _ => shift_ror(rm, amount, carry_in),
    }
}

fn shift_lsl(value: u32, amount: u32, carry_in: bool) -> (u32, bool) {
    match amount {
        0 => (value, carry_in),
        1..=31 => (value << amount, value & (1 << (32 - amount)) != 0),
        32 => (0, value & 1 != 0),
        _ => (0, false),
    }
}

fn shift_lsr(value: u32, amount: u32) -> (u32, bool) {
    match amount {
        0 | 32 => (0, value >> 31 != 0),
        1..=31 => (value >> amount, value & (1 << (amount - 1)) != 0),
        _ => (0, false),
    }
}

fn shift_asr(value: u32, amount: u32) -> (u32, bool) {
    if amount == 0 || amount >= 32 {
        let negative = value >> 31 != 0;
        return (if negative { u32::MAX } else { 0 }, negative);
    }
    (
        ((value as i32) >> amount) as u32,
        value & (1 << (amount - 1)) != 0,
    )
}

fn shift_ror(value: u32, amount: u32, carry_in: bool) -> (u32, bool) {
    if amount == 0 {
        // Immediate ROR #0 is RRX: old C enters bit 31 and bit 0 becomes C.
        return (((carry_in as u32) << 31) | (value >> 1), value & 1 != 0);
    }
    let rotation = amount % 32;
    if rotation == 0 {
        (value, value >> 31 != 0)
    } else {
        (
            value.rotate_right(rotation),
            value & (1 << (rotation - 1)) != 0,
        )
    }
}

/// レジスタ指定シフト。Rs下位8bitが0の場合は全タイプで値とCを保持する。
pub fn barrel_shift_register(rm: u32, shift_type: u8, amount: u32, carry_in: bool) -> (u32, bool) {
    if amount & 0xFF == 0 {
        return (rm, carry_in);
    }
    barrel_shift(rm, shift_type, amount, carry_in)
}

pub fn update_nz(regs: &mut crate::cpu_registers::CpuRegisters, result: u32) {
    regs.set_cpsr_n((result >> 31) & 1 != 0);
    regs.set_cpsr_z(result == 0);
}

/// Evaluate an ARM condition code against CPSR N/Z/C/V flags.
pub fn condition_passed(cpsr: u32, condition: u8) -> bool {
    let n = cpsr & (1 << 31) != 0;
    let z = cpsr & (1 << 30) != 0;
    let c = cpsr & (1 << 29) != 0;
    let v = cpsr & (1 << 28) != 0;
    match condition {
        0x0 => z,
        0x1 => !z,
        0x2 => c,
        0x3 => !c,
        0x4 => n,
        0x5 => !n,
        0x6 => v,
        0x7 => !v,
        0x8 => c && !z,
        0x9 => !c || z,
        0xA => n == v,
        0xB => n != v,
        0xC => !z && n == v,
        0xD => z || n != v,
        0xE => true,
        _ => false,
    }
}

/// Return the effective address and writeback address for ARM pre/post indexing.
pub fn transfer_addresses(base: u32, offset: u32, pre: bool, up: bool) -> (u32, u32) {
    let offset_address = if up {
        base.wrapping_add(offset)
    } else {
        base.wrapping_sub(offset)
    };
    (if pre { offset_address } else { base }, offset_address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsl_shifts() {
        assert_eq!(barrel_shift(0x1, 0, 1, false).0, 0x2);
    }

    #[test]
    fn ror_rrx() {
        let (v, c) = barrel_shift(0x0000_0001, 3, 0, true);
        assert_eq!(v, 0x8000_0000);
        assert!(c);
    }

    #[test]
    fn lsr_zero_is_zero_with_carry() {
        let (v, c) = barrel_shift(0x8000_0000, 1, 0, false);
        assert_eq!(v, 0);
        assert!(c);
    }

    #[test]
    fn register_shift_zero_preserves_value_and_carry() {
        for shift_type in 0..=3 {
            assert_eq!(
                barrel_shift_register(0x81234567, shift_type, 0, true),
                (0x81234567, true)
            );
        }
    }
}
