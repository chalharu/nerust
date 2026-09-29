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

// Long-multiply C flag from the multiplier array (original implementation).
//
// After xMULLS/xMLALS the C flag is not a function of the 64-bit product
// (e.g. UMULL(0xFFFFFFFF, 0x80000000) and UMULL(0x80000000, 0xFFFFFFFF)
// share a product but report different C): it is the carry-out of the
// ARM7TDMI's Booth-recoded carry-save multiplier array, sampled through
// the final ALU add. The datapath below is written from the publicly
// documented organization of that array: radix-4 Booth recoding emitting
// four addends per multiplier cycle, carry-save compression with settled
// 2-bit result latching, early termination on the 33-bit multiplier, and
// a final ALU add whose barrel-shifter carry-out is the observed bit
// (ARM multiply-accumulate patents; Furber, "ARM System-on-Chip
// Architecture"; ARM7TDMI datasheet). No third-party code is used.
// HW truth is pinned by `mull_carry_matches_suite_table` plus the
// mgba-suite multiply-long ROM; model fidelity (result lane) by
// `mull_array_matches_product` below.

/// 33-bit multiplier lane mask (bit 32 = sign extension).
const ARRAY33: u64 = 0x1_FFFF_FFFF;
/// 34-bit recoded-addend lane mask.
const ARRAY34: u64 = 0x3_FFFF_FFFF;

/// Radix-4 Booth factor for one 3-bit multiplier chunk (textbook table).
fn booth_factor(chunk: u64) -> i32 {
    match chunk & 0b111 {
        0b000 | 0b111 => 0,
        0b001 | 0b010 => 1,
        0b011 => 2,
        0b100 => -2,
        _ => -1, // 0b101 | 0b110
    }
}

/// Three-lane carry-save add: modular sum plus the majority carry, which
/// carries double weight (consumed shifted by one below).
fn csa_add(a: u64, b: u64, c: u64) -> (u64, u64) {
    (a ^ b ^ c, (a & b) | (b & c) | (c & a))
}

/// One Booth-recoded addend on 34-bit lanes plus its negation
/// compensation bit. Negative multiples are bitwise inversions (not
/// two's complement): the +1 enters through the CSA carry lane's
/// vacated LSB at compression time.
fn booth_addend(multiplicand: u64, chunk: u64) -> (u64, u64) {
    let factor = booth_factor(chunk);
    let magnitude = multiplicand.wrapping_mul(factor.unsigned_abs() as u64) & ARRAY34;
    if factor < 0 {
        (!magnitude & ARRAY34, 1)
    } else {
        (magnitude, 0)
    }
}

/// 32-bit add with carry-in, returning (sum, carry-out).
fn add32(a: u32, b: u32, carry_in: bool) -> (u32, bool) {
    let total = a as u64 + b as u64 + u64::from(carry_in);
    (total as u32, total >> 32 != 0)
}

/// Sign-extend `value` from `width` bits to 64.
fn sext_from(value: u64, width: u64) -> u64 {
    if width >= 64 || (value >> (width - 1)) & 1 == 0 {
        value
    } else {
        value | (!0u64 << width)
    }
}

/// 33-bit arithmetic right shift by 8 (the multiplier lane step).
fn asr33(value: u64) -> u64 {
    let sign = if value & (1 << 32) != 0 {
        0xFF << 25
    } else {
        0
    };
    ((value & ARRAY33) >> 8) | sign
}

/// One multiplier cycle: compress four recoded addends into the running
/// lanes. Each CSA settles its low 2 bits into the latched tails (later
/// addends are at least 4x larger, so those bits are final); the
/// datapath's sign-extension fold re-enters bit-32/33 evidence plus two
/// accumulate bits at lanes 31/32. Returns (sum, carry, acc_rest).
fn array_compress(sum: u64, carry: u64, addends: [(u64, u64); 4], mut acc: u64) -> (u64, u64, u64) {
    let (mut s, mut c) = (sum, carry);
    let (mut tail_s, mut tail_c) = (0u64, 0u64);
    for (i, &(bits, neg)) in addends.iter().enumerate() {
        s &= ARRAY33;
        c &= ARRAY33;
        let top_c = (c >> 32) & 1;
        let top_b = (bits >> 33) & 1;
        let (ns, nc) = csa_add(s, bits & ARRAY33, c);
        // Booth-negation +1 into the carry lane's vacated LSB.
        let nc = (nc << 1) | neg;
        tail_s |= (ns & 3) << (2 * i);
        tail_c |= (nc & 3) << (2 * i);
        let (mut ns, mut nc) = (ns >> 2, nc >> 2);
        // Sign-extension fold: dropped bit-32/33 evidence plus two
        // accumulate bits re-enter at lanes 31/32.
        ns |= ((acc & 1) + (top_c ^ 1) + (top_b ^ 1)) << 31;
        nc |= (((acc >> 1) & 1) ^ 1) << 32;
        acc >>= 2;
        s = ns;
        c = nc;
    }
    (tail_s | (s << 8), tail_c | (c << 8), acc)
}

/// Full multiplier-array pass over a long multiply. Returns the 64-bit
/// result (always rm*rs+acc; asserted by the fuzz) and the observed C.
fn mull_array(rm: u32, rs: u32, acc: u64, signed: bool) -> (u64, bool) {
    // 34-bit operand lanes: sign-extended for signed ops.
    let extend = |v: u32| -> u64 {
        if signed && v >> 31 != 0 {
            u64::from(v) | 0x3_0000_0000
        } else {
            u64::from(v)
        }
    };
    let mut mult = extend(rs);
    let multiplicand = extend(rm);
    let in_carry = rs & 1 != 0;
    // First-cycle extra input: the bit-0 chunk's addend seeds the lanes,
    // the accumulator seeds the sum lane, its high bits drip-feed below.
    let mut sum = acc;
    let mut carry = if mult & 1 != 0 { !multiplicand } else { 0 };
    let mut acc_rest = acc >> 34;
    // Latch bit 0 and pre-rotate (accounts for the doubled multiplier).
    let mut lat_sum = (sum & 1) as u128;
    let mut lat_carry = (carry & 1) as u128;
    lat_sum = lat_sum.rotate_right(1);
    lat_carry = lat_carry.rotate_right(1);
    sum >>= 1;
    carry >>= 1;
    let mut iters = 0;
    loop {
        let mut addends = [(0u64, 0u64); 4];
        for (i, slot) in addends.iter_mut().enumerate() {
            *slot = booth_addend(multiplicand, mult >> (2 * i));
        }
        let (cs, cc, rest) = array_compress(sum, carry, addends, acc_rest);
        acc_rest = rest;
        // Latch this cycle's settled low 8 bits, then rotate the latches.
        lat_sum |= (cs & 0xFF) as u128;
        lat_carry |= (cc & 0xFF) as u128;
        sum = cs >> 8;
        carry = cc >> 8;
        lat_sum = lat_sum.rotate_right(8);
        lat_carry = lat_carry.rotate_right(8);
        mult = asr33(mult);
        iters += 1;
        let done = if signed {
            mult == ARRAY33 || mult == 0
        } else {
            mult == 0
        };
        if done {
            break;
        }
    }
    lat_sum |= sum as u128;
    lat_carry |= carry as u128;
    // Re-align the latches for the final adds.
    let align = match iters {
        1 => 23,
        2 => 15,
        3 => 7,
        _ => 31,
    };
    lat_sum = lat_sum.rotate_right(align);
    lat_carry = lat_carry.rotate_right(align);
    let ps_hi = (lat_sum >> 64) as u64;
    let pc_hi = (lat_carry >> 64) as u64;
    if iters == 4 {
        let (lo, c0) = add32(ps_hi as u32, pc_hi as u32, in_carry);
        let (hi, _) = add32((ps_hi >> 32) as u32, (pc_hi >> 32) as u32, c0);
        (u64::from(hi) << 32 | u64::from(lo), pc_hi >> 63 != 0)
    } else {
        let (lo, c0) = add32((ps_hi >> 32) as u32, (pc_hi >> 32) as u32, in_carry);
        // Remaining accumulate bits join the low half at 2+8n.
        let shift = 2 + 8 * iters as u64;
        let pc_lo = sext_from(lat_carry as u64, shift);
        let ps_lo = (lat_sum as u64) | acc_rest.wrapping_shl(shift as u32);
        let (hi, _) = add32(ps_lo as u32, pc_lo as u32, c0);
        (u64::from(hi) << 32 | u64::from(lo), pc_hi >> 63 != 0)
    }
}

/// Early-out entry over the fetched low half (acc = pre-add RdLo).
/// Signedness matters: it selects the lane extensions and the early
/// termination point, hence the iteration count that the sampled bit
/// depends on.
pub(crate) fn multiply_carry_lo(rm: u32, rs: u32, accum: u32, signed: bool) -> bool {
    mull_array(rm, rs, u64::from(accum), signed).1
}

/// Full-tick entry over the high half (acc = pre-add RdHi).
pub(crate) fn multiply_carry_hi(rm: u32, rs: u32, accum_hi: u32, signed: bool) -> bool {
    mull_array(rm, rs, u64::from(accum_hi) << 32, signed).1
}

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
                multiply_carry_lo(rm, rs, 0, signed)
            };
            assert_eq!(c, expected_c, "rm={rm:#010X} rs={rs:#010X} signed={signed}");
        }
    }

    /// Model-fidelity fuzz: the array's result lane always equals
    /// rm*rs+acc over an edge grid plus random 32-bit space, for both
    /// accumulate wirings. (The C lane is pinned by
    /// `mull_carry_matches_suite_table` and the mgba-suite ROM.)
    #[test]
    fn mull_array_matches_product() {
        fn xorshift(state: &mut u64) -> u64 {
            let mut x = *state;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *state = x;
            x
        }
        let seeds = [
            0x00000000u32,
            0x00000001,
            0x00000002,
            0x7FFFFFFE,
            0x7FFFFFFF,
            0x80000000,
            0x80000001,
            0xFFFFFFFE,
            0xFFFFFFFF,
            0x12345678,
            0xAAAAAAAA,
            0xDEADBEEF,
        ];
        // Edge grid on both entries (acc wires differ per entry).
        for rm in seeds {
            for rs in seeds {
                for signed in [true, false] {
                    for acc in seeds {
                        if multiply_tick_full(rs, signed) {
                            let (result, _) = mull_array(rm, rs, u64::from(acc) << 32, signed);
                            assert_eq!(
                                result,
                                multiply_64(rm, rs, signed).wrapping_add(u64::from(acc) << 32)
                            );
                        } else {
                            let (result, _) = mull_array(rm, rs, u64::from(acc), signed);
                            assert_eq!(
                                result,
                                multiply_64(rm, rs, signed).wrapping_add(u64::from(acc))
                            );
                        }
                    }
                }
            }
        }
        // Random fuzz across the full 32-bit space, both entries.
        let mut state = 0x9E3779B97F4A7C15u64;
        for _ in 0..200_000 {
            let rm = xorshift(&mut state) as u32;
            let rs = xorshift(&mut state) as u32;
            let acc = xorshift(&mut state) as u32;
            let signed = xorshift(&mut state) & 1 != 0;
            if multiply_tick_full(rs, signed) {
                let (result, _) = mull_array(rm, rs, u64::from(acc) << 32, signed);
                assert_eq!(
                    result,
                    multiply_64(rm, rs, signed).wrapping_add(u64::from(acc) << 32)
                );
            } else {
                let (result, _) = mull_array(rm, rs, u64::from(acc), signed);
                assert_eq!(
                    result,
                    multiply_64(rm, rs, signed).wrapping_add(u64::from(acc))
                );
            }
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
                        let _ = multiply_carry_lo(rm, rs, seed, signed);
                        let _ = multiply_tick_full(rs, signed);
                    }
                }
            }
        }
    }
}
