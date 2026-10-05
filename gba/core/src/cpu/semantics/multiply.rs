use crate::cpu_registers::CpuRegisters;

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
pub(super) fn mull_array(rm: u32, rs: u32, acc: u64, signed: bool) -> (u64, bool) {
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
