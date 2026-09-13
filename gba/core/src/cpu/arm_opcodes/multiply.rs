use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

// Long-multiply carry model.
//
// The ARM7TDMI MULLS/MLALS set C from the Booth multiplier array's final
// carry, not from the 64-bit product (mgba-suite multiply-long pins C=1
// for e.g. SMULL(0, 0x80000000) whose product is 0). The algorithm below
// is a Rust port of NanoBoyAdvance's `MultiplyCarrySimple/Lo/Hi`, itself
// adapted from the original research implementation, and is used under
// its license terms:
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
// ALTERED: ported from C++ to Rust for nerust (wrapping arithmetic made
// explicit; the shift counts are kept mod-32 via wrapping_shl/shr, matching
// the practical behavior of the original). Validated bit-exact against all
// 72 mgba-suite multiply-long C expectations (see `synthetic_mull_carry`
// pins in nerust_gba_rom_test).

pub fn handle(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    // Multiplies take internal cycles (GBATEK 1S+mI); carried in the base
    // below, plus the fetch-stream break (mGBA MUL post-body N32-S32) and
    // the P-ON tick erase (mGBA ARM_WAIT_MUL stall).
    let is_long = (instr >> 23) & 1 != 0;
    if is_long {
        // UMULL/UMLAL/SMULL/SMLAL produce an RdHi:RdLo pair.
        return handle_long(regs, bus, instr);
    }

    handle_short(regs, bus, instr)
}

fn handle_short(regs: &mut CpuRegisters, bus: &mut crate::memory::GbaMemoryBus, instr: u32) -> u32 {
    let a = (instr >> 21) & 1 != 0; // MLA if 1
    let s = (instr >> 20) & 1 != 0;
    let rd = ((instr >> 16) & 0xF) as usize;
    let rn = ((instr >> 12) & 0xF) as usize;
    let rs = ((instr >> 8) & 0xF) as usize;
    let rm = (instr & 0xF) as usize;

    let rs_val = regs.r(rs);
    let rm_val = regs.r(rm);
    let mut result = rm_val.wrapping_mul(rs_val);
    if a {
        result = result.wrapping_add(regs.r(rn));
    }
    // UNPREDICTABLE Rd=R15 (mGBA isa-arm.c skips the write): never let a
    // multiply hijack the PC and trigger a spurious pipeline refill.
    if rd != 15 {
        regs.set_r(rd, result);
    }

    if s {
        crate::cpu::arm_opcodes::helpers::update_nz(regs, result);
    }

    let cycles = multiplier_cycles(rs_val);
    // GBATEK/ARM ARM: MUL=1S+mI, MLA=1S+mI+1I (the 1S is the execute cycle;
    // the opcode fetch is charged separately by the bus). The tick array
    // also breaks the fetch stream (mGBA MUL post-body) and fills prefetch
    // P-ON (mGBA ARM_WAIT_MUL stall on WAIT+m, WAIT=0/1).
    bus.charge_fetch_stream_break();
    bus.erase_for_multiply(cycles + u32::from(a), 4);
    if a { cycles + 2 } else { cycles + 1 }
}

fn handle_long(regs: &mut CpuRegisters, bus: &mut crate::memory::GbaMemoryBus, instr: u32) -> u32 {
    let signed = (instr >> 22) & 1 != 0;
    let accumulate = (instr >> 21) & 1 != 0;
    let set_flags = (instr >> 20) & 1 != 0;
    let rd_hi = ((instr >> 16) & 0xF) as usize;
    let rd_lo = ((instr >> 12) & 0xF) as usize;
    let rs_value = regs.r(((instr >> 8) & 0xF) as usize);
    let rm_value = regs.r((instr & 0xF) as usize);
    let product = multiply_64(rm_value, rs_value, signed);
    let result = if accumulate {
        product.wrapping_add(register_pair(regs, rd_hi, rd_lo))
    } else {
        product
    };
    let hi = (result >> 32) as u32;
    let lo = result as u32;
    // Accumulate seeds must be read before the destination write (the
    // carry model consumes the pre-add RdHi/RdLo, mirroring the HW array).
    let acc_hi = regs.r(rd_hi);
    let acc_lo = regs.r(rd_lo);
    regs.set_r(rd_hi, hi);
    regs.set_r(rd_lo, lo);
    if set_flags {
        regs.set_cpsr_n(hi >> 31 != 0);
        regs.set_cpsr_z(result == 0);
        // N/Z come from the product, but C comes from the Booth array's
        // final carry (see module docs). The array only runs the executed
        // iterations: fully-ticked multiplies use the Hi model, early-out
        // ones the Lo model over the fetched low half.
        let full = multiply_tick_full(rs_value, signed);
        let carry = if full {
            multiply_carry_hi(
                rm_value,
                rs_value,
                if accumulate { acc_hi } else { 0 },
                signed,
            )
        } else {
            multiply_carry_lo(rm_value, rs_value, if accumulate { acc_lo } else { 0 })
        };
        regs.set_cpsr_c(carry);
    }
    // GBATEK: UMULL/SMULL=1S+mI+1I, UMLAL/SMLAL=1S+mI+2I.
    let ticks = multiplier_cycles_long(rs_value, signed);
    // mGBA long-MUL post-body + tick erase (WAIT: xMLAL 2+m, xMULL 1+m).
    bus.charge_fetch_stream_break();
    bus.erase_for_multiply(ticks + 1 + u32::from(accumulate), 4);
    ticks + 2 + u32::from(accumulate)
}

fn multiply_64(left: u32, right: u32, signed: bool) -> u64 {
    if signed {
        (left as i32 as i64).wrapping_mul(right as i32 as i64) as u64
    } else {
        u64::from(left).wrapping_mul(u64::from(right))
    }
}

fn register_pair(regs: &CpuRegisters, hi: usize, lo: usize) -> u64 {
    (u64::from(regs.r(hi)) << 32) | u64::from(regs.r(lo))
}

pub(crate) fn multiplier_cycles(rs_val: u32) -> u32 {
    // Short MUL/MLA: zero-or-one rule (32-bit truncated result).
    multiplier_cycles_long(rs_val, true)
}

/// GBATEK ARM Multiply Long: m counts Rs top bits that are "all zero"
/// (UMULL/UMLAL) or "all zero or all one" (SMULL/SMLAL).
fn multiplier_cycles_long(rs_val: u32, signed: bool) -> u32 {
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
fn multiply_tick_full(rs_val: u32, signed: bool) -> bool {
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
fn multiply_carry_lo(rm: u32, rs: u32, accum: u32) -> bool {
    // Set low bit of multiplicand to cause negation to invert the upper
    // bits. This bit cannot propagate to the resulting carry bit.
    let multiplicand = rm | 1;
    // Optimized first iteration.
    let mut booth = ((rs.wrapping_shl(31)) as i32 >> 31) as u32;
    let mut carry = multiplicand.wrapping_mul(booth);
    let mut sum = carry.wrapping_add(accum);
    let mut acc = accum;
    // Process 8 multiplier bits using 4 booth iterations per group.
    // The loop is bounded: partial-tick callers guarantee uniform top
    // bits, so booth always converges within 3 groups (after the shift-7
    // group booth replicates Rs[24] across the top, matching Rs whenever
    // the top 8 bits are uniform); full-tick multipliers never reach this
    // path (they use the Hi model). The bound keeps this a total function
    // even for unreachable inputs (full-tick Rs here), where the C++
    // original would spin on the shift-count wraparound. Wrapping keeps
    // the mod-32 shift semantics of the original in any case.
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
fn multiply_carry_hi(rm: u32, rs: u32, accum_hi: u32, signed: bool) -> bool {
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
    use crate::cpu_registers::CpuRegisters;
    use crate::memory::GbaMemoryBus;

    #[test]
    fn mul_simple() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        regs.set_r(1, 3);
        regs.set_r(2, 4);
        // MUL R0, R1, R2 -> E0000290? Actually MUL R0,R1,R2 = E0000192?
        // Encoding: 0xE0000291? Let's use MUL R0,R1,R2 = E0000091 with Rs=1, Rm=2
        // 0xE0000091: cond E, 000,000,0,0,0, Rd=0, Rn=0, Rs=1, 1001, Rm=2
        // Simplified: Use our handler directly
        regs.set_r(0, 0);
        let instr = 0xE0000291u32; // MUL R0, R2, R1 (Rd=0, Rs=2, Rm=1)
        handle(&mut regs, &mut bus, instr);
        assert_eq!(regs.r(0), 12);
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
            (0x80000000, 0x80000001, false, true),
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
        // via handle_long, which selects Hi there) and non-zero accumulate
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
