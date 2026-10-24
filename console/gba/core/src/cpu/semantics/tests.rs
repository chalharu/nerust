use super::multiply::{
    mull_array, multiply_64, multiply_carry_hi, multiply_carry_lo, multiply_tick_full,
};
use super::shifter::{barrel_shift, barrel_shift_register};

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
