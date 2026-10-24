//! Pure CPU semantic helpers owned by the unified micro-op engine:
//! the barrel shifter, NZCV/condition evaluation, block start address,
//! the MRS/MSR class predicate, and the multiply timing/Booth-array
//! carry model. No bus access, no instruction dispatch.

use crate::cpu_registers::CpuRegisters;

/// Barrel shifter: LSL/LSR/ASR/ROR + RRX
/// shift_type: 0=LSL, 1=LSR, 2=ASR, 3=ROR
pub(crate) fn barrel_shift(rm: u32, shift_type: u8, amount: u32, carry_in: bool) -> (u32, bool) {
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
        // Register ROR with a multiple of 32 (but nonzero low byte): the
        // 5-bit rotate amount is 0, so the result is unchanged, but carry
        // is set to bit 31 (ARM ARM; jsmolka_thumb pins this).
        // Only a zero low *byte* preserves carry (handled above).
        (value, value >> 31 != 0)
    } else {
        (
            value.rotate_right(rotation),
            value & (1 << (rotation - 1)) != 0,
        )
    }
}

/// レジスタ指定シフト。Rs下位8bitが0の場合は全タイプで値とCを保持する。
pub(crate) fn barrel_shift_register(
    rm: u32,
    shift_type: u8,
    amount: u32,
    carry_in: bool,
) -> (u32, bool) {
    if amount & 0xFF == 0 {
        return (rm, carry_in);
    }
    barrel_shift(rm, shift_type, amount, carry_in)
}

pub(crate) fn update_nz(regs: &mut CpuRegisters, result: u32) {
    regs.set_cpsr_n((result >> 31) & 1 != 0);
    regs.set_cpsr_z(result == 0);
}

/// Evaluate an ARM condition code against CPSR N/Z/C/V flags.
pub(crate) fn condition_passed(cpsr: u32, condition: u8) -> bool {
    // AL/NV decide without flag extraction (the overwhelmingly common
    // ARM case is unconditional execution).
    if condition == 0xE {
        return true;
    }
    if condition == 0xF {
        return false;
    }
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

/// First word address of an ARM block transfer (P/U select IA/IB/DA/DB).
pub(crate) fn start_address(base: u32, count: u32, pre: bool, up: bool) -> u32 {
    match (up, pre) {
        (true, true) => base.wrapping_add(4),
        (true, false) => base,
        (false, true) => base.wrapping_sub(count * 4),
        (false, false) => base.wrapping_sub(count * 4).wrapping_add(4),
    }
}

/// MRS/MSR class predicate shared by the ARM decoder and micro-op expansion.
pub(crate) fn is_psr_transfer(instr: u32) -> bool {
    (instr & 0x0FBF0FFF) == 0x010F0000
        || (instr & 0x0FB0FFF0) == 0x0120F000
        || (instr & 0x0FB0F000) == 0x0320F000
}
