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

// Long-multiply carry model: C comes from the Booth array's final carry, not the product.
// Ported algorithm used under its license terms below.
//
//   Multiplication carry flag algorithm has been altered from its original
//   form. However, they remain under their original license terms.
//
//   Copyright (C) 2024 zaydlang, calc84maniac
//
//   This software is provided 'as-is', without any express or implied
//   warranty. In no event will the authors be held liable for any damages
//   arising from the use of this software.
//
//   Permission is granted to anyone to use this software for any purpose,
//   including commercial applications, and to alter it and redistribute it
//   freely, subject to the following restrictions:
//
//   1. The origin of this software must not be misrepresented; you must not
//      claim that you wrote the original software. If you use this software
//      in a product, an acknowledgment in the product documentation would be
//      appreciated but is not required.
//   2. Altered source versions must be plainly marked as such, and must not
//      be misrepresented as being the original software.
//   3. This notice may not be removed or altered from any source
//      distribution.
//
// ALTERED: moved from `arm_opcodes::multiply` into the unified `semantics`
// module during micro-op/legacy unification (no algorithmic change).
// Validated bit-exact against all 72 mgba-suite multiply-long C
// expectations (see `mull_carry_matches_suite_table` below and the
// `synthetic_mull_carry` pins in nerust_gba_rom_test).

pub(crate) fn multiply_64(left: u32, right: u32, signed: bool) -> u64 {
    if signed {
        (left as i32 as i64).wrapping_mul(right as i32 as i64) as u64
    } else {
        (u64::from(left)).wrapping_mul(u64::from(right))
    }
}

pub(crate) fn register_pair(regs: &CpuRegisters, hi: usize, lo: usize) -> u64 {
    (u64::from(regs.r(hi)) << 32) | u64::from(regs.r(lo))
}

pub(crate) fn multiplier_cycles(rs_val: u32) -> u32 {
    // Short MUL/MLA: zero-or-one rule (32-bit truncated result).
    multiplier_cycles_long(rs_val, true)
}

/// GBATEK ARM Multiply Long: m counts Rs top bits that are "all zero"
/// (UMULL/UMLAL) or "all zero or all one" (SMULL/SMLAL).
pub(crate) fn multiplier_cycles_long(rs_val: u32, signed: bool) -> u32 {
    let top = |mask: u32, ones: u32| rs_val & mask == 0 || (signed && rs_val & mask == ones);
    if top(0xFFFFFF00, 0xFFFFFF00) {
        1
    } else if top(0xFFFF0000, 0xFFFF0000) {
        2
    } else if top(0xFF000000, 0xFF000000) {
        3
    } else {
        4
    }
}

/// Whether the multiplier array runs all iterations (no early-out).
/// Pure predicate mirroring the tick loop: `true` selects the Hi carry
/// model, `false` the Lo model. Timing is unchanged (cycle counts still
/// come from `multiplier_cycles_long`).
pub(crate) fn multiply_tick_full(rs_val: u32, signed: bool) -> bool {
    let mut mask = 0xFFFFFF00u32;
    loop {
        let m = rs_val & mask;
        if m == 0 {
            break;
        }
        if signed && m == mask {
            break;
        }
        mask = mask.wrapping_shl(8);
        if mask == 0 {
            break;
        }
    }
    mask == 0
}

/// Booth-array carry for early-out (partial) multiplies, over the low
/// half. `accum` is the pre-add RdLo (0 without accumulate).
pub(crate) fn multiply_carry_lo(rm: u32, rs: u32, accum: u32) -> bool {
    // Set low bit of multiplicand to cause negation to invert the upper
    // bits. This bit cannot propagate to the resulting carry bit.
    let multiplicand = rm | 1;
    // Optimized first iteration.
    let mut booth = ((rs.wrapping_shl(31)) as i32 >> 31) as u32;
    let mut carry = multiplicand.wrapping_mul(booth);
    let mut sum = carry.wrapping_add(accum);
    let mut acc = accum;
    // Loop is bounded: partial-tick inputs converge within 3 groups, keeping this total.
    // Full-tick inputs never reach this path (they use the Hi model).
    let mut shift = 29i32;
    for _ in 0..4 {
        for _ in 0..4 {
            // Next booth factor (-2 to 2, scaled).
            let next = ((rs.wrapping_shl(shift as u32)) as i32).wrapping_shr(shift as u32) as u32;
            shift -= 2;
            let factor = next.wrapping_sub(booth);
            booth = next;
            let addend = multiplicand.wrapping_mul(factor);
            // Accumulate addend with carry-save add.
            acc ^= carry ^ addend;
            sum = sum.wrapping_add(addend);
            carry = sum.wrapping_sub(acc);
        }
        if booth == rs {
            break;
        }
    }
    // Carry flag comes from bit 31 of carry-save adder's final carry.
    carry >> 31 != 0
}

/// Booth-array carry for fully-ticked multiplies, over the high half.
/// `accum_hi` is the pre-add RdHi (0 without accumulate).
pub(crate) fn multiply_carry_hi(rm: u32, rs: u32, accum_hi: u32, signed: bool) -> bool {
    // Only last 3 booth iterations are relevant to output carry.
    // Reduce scale of both inputs to get upper bits of 64-bit booth addends
    // in upper bits of 32-bit values, while handling sign extension.
    let (multiplicand, multiplier) = if signed {
        ((rm as i32 >> 6) as u32, (rs as i32 >> 26) as u32)
    } else {
        (rm >> 6, rs >> 26)
    };
    // Set low bit of multiplicand to cause negation to invert the upper
    // bits. This bit cannot propagate to the resulting carry bit.
    let multiplicand = multiplicand | 1;
    // Pre-populate magic bit 61 for carry.
    let carry = !accum_hi & 0x20000000;
    // Pre-populate magic bits 63-60 for accum (with carry magic pre-added).
    let mut accum = accum_hi.wrapping_sub(0x08000000);
    // Factors for last 3 booth iterations.
    let booth0 = ((multiplier.wrapping_shl(27)) as i32).wrapping_shr(27) as u32;
    let booth1 = ((multiplier.wrapping_shl(29)) as i32).wrapping_shr(29) as u32;
    let booth2 = ((multiplier.wrapping_shl(31)) as i32).wrapping_shr(31) as u32;
    let factor0 = multiplier.wrapping_sub(booth0);
    let factor1 = booth0.wrapping_sub(booth1);
    let factor2 = booth1.wrapping_sub(booth2);
    // Scaled value of 3rd-last booth addend.
    let mut addend = multiplicand.wrapping_mul(factor2);
    // Finalize bits 61-60 of accum magic using its sign.
    accum = accum.wrapping_sub(addend & 0x10000000);
    // Scaled value of 2nd-last booth addend.
    addend = multiplicand.wrapping_mul(factor1);
    // Finalize bits 63-62 of accum magic using its sign.
    accum = accum.wrapping_sub(addend & 0x40000000);
    // Carry from carry-save add in bit 61, propagated to bit 62.
    let mut sum = accum.wrapping_add(addend & 0x20000000);
    // Subtract out carry magic to get actual accum magic.
    accum = accum.wrapping_sub(carry);
    // Scaled value of last booth addend; add to bit 62 and propagate.
    addend = multiplicand.wrapping_mul(factor0);
    sum = sum.wrapping_add(addend & 0x40000000);
    // Cancel out accum magic bit 63 to get carry bit 63.
    (sum ^ accum) >> 31 != 0
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

    #[test]
    fn register_ror_multiple_of_32_sets_carry_from_bit31() {
        // Register ROR by 32/64/...: result unchanged, carry = bit 31
        // (NOT preserved). Only a zero low byte preserves carry.
        assert_eq!(
            barrel_shift_register(0x81234567, 3, 32, false),
            (0x81234567, true)
        );
        assert_eq!(
            barrel_shift_register(0x01234567, 3, 64, true),
            (0x01234567, false)
        );
    }

    #[test]
    fn mull_tick_full_predicate() {
        // Early-out (partial) multipliers...
        assert!(!multiply_tick_full(0x00000000, true));
        assert!(!multiply_tick_full(0x00000001, true));
        assert!(!multiply_tick_full(0xFFFFFFFF, true)); // all-ones (signed)
        assert!(!multiply_tick_full(0x00000000, false));
        assert!(!multiply_tick_full(0x00000001, false));
        // ...vs fully-ticked ones.
        assert!(multiply_tick_full(0x80000000, true));
        assert!(multiply_tick_full(0x80000001, true));
        assert!(multiply_tick_full(0x7FFFFFFF, false));
        assert!(multiply_tick_full(0xFFFFFFFF, false)); // not all-ones (unsigned)
        assert!(multiply_tick_full(0x80000000, false));
    }

    #[test]
    fn mull_carry_matches_suite_table() {
        // (rm, rs, signed, expected_c) from mgba-suite multiply-long.c
        // (CPSR bit 29 of outSmullCpsr/outUmullCpsr).
        let vectors: [(u32, u32, bool, bool); 72] = [
            (0x00000000, 0x00000000, true, false),
            (0x00000000, 0x00000000, false, false),
            (0x00000001, 0x00000000, true, false),
            (0x00000001, 0x00000000, false, false),
            (0xFFFFFFFF, 0x00000000, true, false),
            (0xFFFFFFFF, 0x00000000, false, false),
            (0x7FFFFFFF, 0x00000000, true, false),
            (0x7FFFFFFF, 0x00000000, false, false),
            (0x80000000, 0x00000000, true, false),
            (0x80000000, 0x00000000, false, false),
            (0x80000001, 0x00000000, true, false),
            (0x80000001, 0x00000000, false, false),
            (0x00000000, 0x00000001, true, false),
            (0x00000000, 0x00000001, false, false),
            (0x00000001, 0x00000001, true, false),
            (0x00000001, 0x00000001, false, false),
            (0xFFFFFFFF, 0x00000001, true, false),
            (0xFFFFFFFF, 0x00000001, false, false),
            (0x7FFFFFFF, 0x00000001, true, false),
            (0x7FFFFFFF, 0x00000001, false, false),
            (0x80000000, 0x00000001, true, false),
            (0x80000000, 0x00000001, false, false),
            (0x80000001, 0x00000001, true, false),
            (0x80000001, 0x00000001, false, false),
            (0x00000000, 0xFFFFFFFF, true, false),
            (0x00000000, 0xFFFFFFFF, false, false),
            (0x00000001, 0xFFFFFFFF, true, false),
            (0x00000001, 0xFFFFFFFF, false, false),
            (0xFFFFFFFF, 0xFFFFFFFF, true, false),
            (0xFFFFFFFF, 0xFFFFFFFF, false, true),
            (0x7FFFFFFF, 0xFFFFFFFF, true, false),
            (0x7FFFFFFF, 0xFFFFFFFF, false, true),
            (0x80000000, 0xFFFFFFFF, true, false),
            (0x80000000, 0xFFFFFFFF, false, false),
            (0x80000001, 0xFFFFFFFF, true, false),
            (0x80000001, 0xFFFFFFFF, false, false),
            (0x00000000, 0x7FFFFFFF, true, false),
            (0x00000000, 0x7FFFFFFF, false, false),
            (0x00000001, 0x7FFFFFFF, true, false),
            (0x00000001, 0x7FFFFFFF, false, false),
            (0xFFFFFFFF, 0x7FFFFFFF, true, true),
            (0xFFFFFFFF, 0x7FFFFFFF, false, true),
            (0x7FFFFFFF, 0x7FFFFFFF, true, false),
            (0x7FFFFFFF, 0x7FFFFFFF, false, false),
            (0x80000000, 0x7FFFFFFF, true, true),
            (0x80000000, 0x7FFFFFFF, false, true),
            (0x80000001, 0x7FFFFFFF, true, true),
            (0x80000001, 0x7FFFFFFF, false, true),
            (0x00000000, 0x80000000, true, true),
            (0x00000000, 0x80000000, false, false),
            (0x00000001, 0x80000000, true, true),
            (0x00000001, 0x80000000, false, false),
            (0xFFFFFFFF, 0x80000000, true, false),
            (0xFFFFFFFF, 0x80000000, false, true),
            (0x7FFFFFFF, 0x80000000, true, true),
            (0x7FFFFFFF, 0x80000000, false, false),
            (0x80000000, 0x80000000, true, false),
            (0x80000000, 0x80000000, false, true),
            (0x80000001, 0x80000000, true, false),
            (0x80000001, 0x80000000, false, true),
            (0x00000000, 0x80000001, true, true),
            (0x00000000, 0x80000001, false, false),
            (0x00000001, 0x80000001, true, true),
            (0x00000001, 0x80000001, false, false),
            (0xFFFFFFFF, 0x80000001, true, false),
            (0xFFFFFFFF, 0x80000001, false, true),
            (0x7FFFFFFF, 0x80000001, true, true),
            (0x7FFFFFFF, 0x80000001, false, false),
            (0x80000000, 0x80000001, true, false),
            (0x80000001, 0x80000001, false, true),
            (0x80000001, 0x80000001, true, false),
            (0x80000001, 0x80000001, false, true),
        ];
        for (rm, rs, signed, expected_c) in vectors {
            let full = multiply_tick_full(rs, signed);
            let c = if full {
                multiply_carry_hi(rm, rs, 0, signed)
            } else {
                multiply_carry_lo(rm, rs, 0)
            };
            assert_eq!(c, expected_c, "rm={rm:#010X} rs={rs:#010X} signed={signed}");
        }
    }

    #[test]
    fn mull_carry_terminates_on_grid() {
        // The carry models must terminate for every input combination,
        // including full-tick multipliers fed to the Lo path (unreachable
        // via apply_mul_long, which selects Hi there) and non-zero accumulate
        // seeds: no hangs, no shift panics.
        let rms = [
            0x00000000u32,
            0x00000001,
            0x7FFFFFFF,
            0x80000000,
            0x80000001,
            0xFFFFFFFF,
            0x12345678,
            0xAAAAAAAA,
        ];
        let rss = rms;
        let seeds = [0x00000000u32, 0x00000001, 0x80000000, 0xDEADBEEF];
        for rm in rms {
            for rs in rss {
                for signed in [true, false] {
                    for seed in seeds {
                        let _ = multiply_carry_hi(rm, rs, seed, signed);
                        let _ = multiply_carry_lo(rm, rs, seed);
                        let _ = multiply_tick_full(rs, signed);
                    }
                }
            }
        }
    }
}
