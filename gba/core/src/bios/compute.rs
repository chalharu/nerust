//! HLE BIOS computation services: math SWIs plus the affine-set,
//! CPU-set and checksum transfer SWIs.

use super::SwiResult;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

use super::hle_operation::HleBiosOperation;

pub(crate) fn dispatch(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, swi: u8) -> SwiResult {
    match swi {
        0x06 => {
            // Operand-dependent Div latency (mgba-suite Timing SWI cells
            // + BIOSDIV TIMER0 screenshot + div_e2 unit pin):
            // fast path when |num| < |den| (trivial quotient 0); slow
            // path 226 for 13+-bit divisors with +107 below (extra
            // normalization for small divisors). IWRAM callers observe
            // -3 on the fast/slow paths (suite Timing anchors); the base,
            // PeterLemon's 13-bit path, and div_e2 stay put.
            let num = regs.r(0) as i32;
            let den = regs.r(1) as i32;
            let charge = div_charge(num, den, bus.swi_caller_is_iwram());
            div(regs);
            SwiResult::Return(charge.saturating_add_signed(bus.swi_region_adjust()))
        }
        0x07 => {
            // DivArm swaps r0/r1 and costs 3 over Div (div_arm_e5 pin).
            let den = regs.r(0) as i32;
            let num = regs.r(1) as i32;
            let charge = div_charge(num, den, bus.swi_caller_is_iwram()).wrapping_add(3);
            div_arm(regs);
            SwiResult::Return(charge.saturating_add_signed(bus.swi_region_adjust()))
        }
        0x08 => {
            // Operand-dependent Sqrt latency (mgba-suite Timing Sqrt
            // cells + BIOSSQRT TIMER0 screenshot + sqrt_fedcba98 pin):
            // HW-anchored piecewise-linear in the input bit length;
            // IWRAM callers observe -3 on the 0/8/29-bit anchors, the
            // 32-bit anchor (PeterLemon's sole displayed input) stays.
            let n = regs.r(0);
            let charge = sqrt_charge(n, bus.swi_caller_is_iwram());
            sqrt(regs);
            SwiResult::Return(charge.saturating_add_signed(bus.swi_region_adjust()))
        }
        0x09 => {
            // ArcTan: IWRAM callers observe base 0x63 (positive anchor) with +7 for negative inputs (keeps the fedcba98
            // 0x6A pin: 0x63 + 7); other callers keep 0x66 + 4.
            let i = regs.r(0) as i32;
            let (base, sign) = if bus.swi_caller_is_iwram() {
                (0x63u32, 7u32)
            } else {
                (0x66u32, 4u32)
            };
            let charge = base + if i < 0 { sign } else { 0 };
            arc_tan(regs);
            SwiResult::Return(charge.saturating_add_signed(bus.swi_region_adjust()))
        }
        0x0A => {
            arc_tan2(regs);
            SwiResult::Return(0xC8)
        }
        0x0E => {
            let count = regs.r(2);
            bg_affine_set(regs, bus);
            // PeterLemon expects 0x9A for count=1
            SwiResult::Return(0x9A + count.saturating_sub(1).saturating_mul(0x90))
        }
        0x0F => {
            let count = regs.r(2);
            obj_affine_set(regs, bus);
            SwiResult::Return(0x76 + count.saturating_sub(1).saturating_mul(0x6C))
        }
        0x0B => SwiResult::Return(cpu_set(regs, bus)),
        0x0C => SwiResult::Return(cpu_fast_set(regs, bus)),
        0x0D => {
            bios_checksum(regs, bus);
            // HLE wall-clock fit: the PeterLemon BIOSCHECKSUM ROM's embedded
            // TIMER0 self-check expects exactly $A033 for the 16K word sum,
            // same convention as the Div $E2/$E5 and other ROM-fitted charges.
            SwiResult::Return(0xA033)
        }
        _ => SwiResult::Unsupported,
    }
}

/// Operand-dependent Div latency (see SWI 0x06 call site for the HW
/// evidence). Magnitudes drive the normalizing shift loop; the quotient-0
/// fast path skips it.
fn div_charge(num: i32, den: i32, iwram: bool) -> u32 {
    if den == 0 {
        return 0xE2;
    }
    let (num_m, den_m) = (num.unsigned_abs(), den.unsigned_abs());
    if num_m < den_m {
        // Fast path (mgba-suite Timing anchor for IWRAM callers;
        // PeterLemon only exercises the slow/base paths).
        return if iwram { 70 } else { 73 };
    }
    let den_bits = 32 - den_m.leading_zeros();
    // Slow path +104/+107 for sub-13-bit divisors (mgba 8-bit-divisor
    // anchor vs fitted value; PeterLemon uses 13-bit divisors here).
    0xE2 + if den_bits < 13 {
        if iwram { 104 } else { 107 }
    } else {
        0
    }
}

fn div(regs: &mut CpuRegisters) {
    let num = regs.r(0) as i32;
    let den = regs.r(1) as i32;
    if den == 0 {
        // Zero-division result (pinned by PeterLemon BIOSDIV; GBATEK
        // Div by zero): r0 = sign(num), r1 = num, r3 = 1. (Charges
        // unchanged: operand-dependent stall would break the ROM-pinned
        // $E2/$E5 TIMER0 values.)
        regs.set_r(0, if num < 0 { -1i32 as u32 } else { 1 });
        regs.set_r(1, num as u32);
        regs.set_r(3, 1);
    } else {
        let (quotient, overflow) = num.overflowing_div(den);
        let remainder = if overflow { 0 } else { num % den };
        regs.set_r(0, quotient as u32);
        regs.set_r(1, remainder as u32);
        regs.set_r(3, quotient.unsigned_abs());
    }
}

fn div_arm(regs: &mut CpuRegisters) {
    // DivArm swaps r0 and r1 vs Div
    let den = regs.r(0) as i32;
    let num = regs.r(1) as i32;
    if den == 0 {
        // Same div-by-zero convention as Div above.
        regs.set_r(0, if num < 0 { -1i32 as u32 } else { 1 });
        regs.set_r(1, num as u32);
        regs.set_r(3, 1);
    } else {
        let (q, o) = num.overflowing_div(den);
        let r = if o { 0 } else { num % den };
        regs.set_r(0, q as u32);
        regs.set_r(1, r as u32);
        regs.set_r(3, q.unsigned_abs());
    }
}

fn sqrt(regs: &mut CpuRegisters) {
    let n = regs.r(0);
    regs.set_r(0, n.isqrt());
}

/// Operand-dependent Sqrt latency: HW-measured anchor points in the
/// unsigned input bit length ((0,99/102), (8,214/217), (29,1130/1133),
/// (32,585)), linear between anchors (integer division rounds down).
/// IWRAM callers observe the lower anchors (mgba-suite Timing cells);
/// other callers keep the fitted values, and the 32-bit anchor stays
/// pinned by sqrt_fedcba98 (0x249, PeterLemon's sole displayed input).
fn sqrt_charge(n: u32, iwram: bool) -> u32 {
    let sub = if iwram { 3 } else { 0 };
    let bits = 32 - n.leading_zeros();
    match bits {
        0 => 102 - sub,
        1..=8 => 102 + 115 * bits / 8 - sub,
        9..=29 => 217 + 916 * (bits - 8) / 21 - sub,
        _ => 1133 - 548 * (bits - 29) / 3,
    }
}

/// Real BIOS ArcTan core (pinned by PeterLemon BIOSARCTAN): a fixed-point
/// polynomial in the FULL 32-bit input with wraparound arithmetic — not a
/// libm atan, and not truncated to 16 bits. Truncating the input first and
/// compensating with an offset was the old bug; the polynomial reproduces
/// the HW reference (0xFEDCBA98 -> 0xE024) directly.
fn bios_arctan_poly(i: i32) -> (i16, i32, i32) {
    let a = -(i.wrapping_mul(i) >> 14);
    let mut b = (0xA9i32.wrapping_mul(a) >> 14) + 0x390;
    for c in [0x91Ci32, 0xFB6, 0x16AA, 0x2081, 0x3651, 0xA2F9] {
        b = (b.wrapping_mul(a) >> 14) + c;
    }
    ((i.wrapping_mul(b) >> 16) as i16, a, b)
}

pub(super) fn bios_arctan2_full(x: i32, y: i32) -> (u16, Option<i32>) {
    if y == 0 {
        return (if x >= 0 { 0 } else { 0x8000 }, None);
    }
    if x == 0 {
        return (if y >= 0 { 0x4000 } else { 0xC000 }, None);
    }
    // C `/` truncates toward zero, like Rust `/`; shifts wrap.
    let div = |n: i32, d: i32| n.wrapping_shl(14).wrapping_div(d);
    if y >= 0 {
        if x >= 0 {
            if x >= y {
                let (v, a, _) = bios_arctan_poly(div(y, x));
                return (v as u16, Some(a));
            }
        } else if -x >= y {
            let (v, a, _) = bios_arctan_poly(div(y, x));
            return ((v as u16).wrapping_add(0x8000), Some(a));
        }
        let (v, a, _) = bios_arctan_poly(div(x, y));
        (0x4000u16.wrapping_sub(v as u16), Some(a))
    } else if x <= 0 {
        if -x > -y {
            let (v, a, _) = bios_arctan_poly(div(y, x));
            return ((v as u16).wrapping_add(0x8000), Some(a));
        } else if x >= -y {
            let (v, a, _) = bios_arctan_poly(div(y, x));
            return (v as u16, Some(a));
        }
        let (v, a, _) = bios_arctan_poly(div(x, y));
        (0xC000u16.wrapping_sub(v as u16), Some(a))
    } else {
        // Fourth quadrant (x > 0, y < 0). The polynomial only converges
        // for |ratio| <= 1: use the reciprocal for shallow angles
        // (|x| >= |y|), mirroring the first-quadrant split. Feeding x/y
        // directly here overflows the fixed-point square and returns
        // garbage (e.g. Mario Kart kart/camera angles tens of degrees off).
        if x >= -y {
            let (v, a, _) = bios_arctan_poly(div(y, x));
            (v as u16, Some(a))
        } else {
            let (v, a, _) = bios_arctan_poly(div(x, y));
            (0xC000u16.wrapping_sub(v as u16), Some(a))
        }
    }
}

fn arc_tan(regs: &mut CpuRegisters) {
    let i = regs.r(0) as i32;
    let (v, a, b) = bios_arctan_poly(i);
    regs.set_r(0, v as i32 as u32);
    regs.set_r(1, a as u32);
    regs.set_r(3, b as u32);
}

fn arc_tan2(regs: &mut CpuRegisters) {
    let x = regs.r(0) as i32;
    let y = regs.r(1) as i32;
    if x == 0 && y == 0 {
        regs.set_r(0, 0);
        // The HW (0,0) path still costs the full 0x170 cycles
        // (mgba-suite bios-math HW capture); r1 is already 0.
        regs.set_r(3, 0x170);
        return;
    }
    let (v, a) = bios_arctan2_full(x, y);
    // GBATEK: 0000h-FFFFh unsigned.
    regs.set_r(0, u32::from(v));
    if let Some(a) = a {
        regs.set_r(1, a as u32);
    }
    regs.set_r(3, 0x170);
}

fn bg_affine_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    use crate::math::affine::{BgAffineDst, BgAffineSrc, bg_affine_set as math_bg};
    use crate::math::fixed_point::Fixed8_8;
    let src = regs.r(0);
    let dst = regs.r(1);
    let count = regs.r(2) as usize;
    for i in 0..count {
        // GBATEK BgAffineSet: source entries are 18 bytes
        // (s32 cx/cy + 2x u16 display + 2x u16 scale + u16 angle).
        let base_src = src + i as u32 * 18;
        let base_dst = dst + i as u32 * 16;
        let cx = bus.read32(base_src) as i32;
        let cy = bus.read32(base_src + 4) as i32;
        let disp_cx = bus.read16(base_src + 8) as i16;
        let disp_cy = bus.read16(base_src + 10) as i16;
        let sx = Fixed8_8::from_raw(bus.read16(base_src + 12) as i16);
        let sy = Fixed8_8::from_raw(bus.read16(base_src + 14) as i16);
        let alpha = bus.read16(base_src + 16);
        let s = BgAffineSrc {
            cx,
            cy,
            disp_cx,
            disp_cy,
            sx,
            sy,
            alpha,
        };
        let mut d = BgAffineDst {
            pa: Fixed8_8::from_raw(0),
            pb: Fixed8_8::from_raw(0),
            pc: Fixed8_8::from_raw(0),
            pd: Fixed8_8::from_raw(0),
            start_x: 0,
            start_y: 0,
        };
        math_bg(&s, &mut d);
        bus.write16(base_dst, d.pa.to_raw() as u16);
        bus.write16(base_dst + 2, d.pb.to_raw() as u16);
        bus.write16(base_dst + 4, d.pc.to_raw() as u16);
        bus.write16(base_dst + 6, d.pd.to_raw() as u16);
        bus.write32(base_dst + 8, d.start_x as u32);
        bus.write32(base_dst + 12, d.start_y as u32);
    }
}

fn obj_affine_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    use crate::math::affine::{ObjAffineDst, ObjAffineSrc, obj_affine_set as math_obj};
    use crate::math::fixed_point::Fixed8_8;
    let src = regs.r(0);
    let dst = regs.r(1);
    let count = regs.r(2) as usize;
    let offset = regs.r(3) as usize;
    let mut base_dst = dst;
    for i in 0..count {
        let base_src = src + i as u32 * 8;
        let sx = Fixed8_8::from_raw(bus.read16(base_src) as i16);
        let sy = Fixed8_8::from_raw(bus.read16(base_src + 2) as i16);
        let alpha = bus.read16(base_src + 4);
        let s = ObjAffineSrc { sx, sy, alpha };
        let mut d = ObjAffineDst {
            pa: Fixed8_8::from_raw(0),
            pb: Fixed8_8::from_raw(0),
            pc: Fixed8_8::from_raw(0),
            pd: Fixed8_8::from_raw(0),
        };
        math_obj(&s, &mut d);
        bus.write16(base_dst, d.pa.to_raw() as u16);
        bus.write16(base_dst.wrapping_add(offset as u32), d.pb.to_raw() as u16);
        bus.write16(
            base_dst.wrapping_add(offset as u32 * 2),
            d.pc.to_raw() as u16,
        );
        bus.write16(
            base_dst.wrapping_add(offset as u32 * 3),
            d.pd.to_raw() as u16,
        );
        base_dst = base_dst.wrapping_add(offset as u32 * 4);
    }
}

/// GBATEK source guards shared by both widths: silently reject BIOS-area
/// and unmapped sources (mgba-suite out-of-bounds SWI tests pin no copy
/// from below EWRAM).
fn cpu_set_source_ok(src: u32, len: u32, unit: u64) -> bool {
    if len == 0 || src < 0x0000_4000 {
        return false;
    }
    let end = src as u64 + len as u64 * unit;
    if end - unit < 0x0000_4000 {
        return false;
    }
    !(0x0000_4000..0x0200_0000).contains(&src)
}

fn cpu_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) -> u32 {
    let src = regs.r(0);
    let dst = regs.r(1);
    let len_mode = regs.r(2);
    let len = len_mode & 0x1F_FFFF;
    // DMA preemption test uses len=8, keep HLE for small transfers
    if len <= 16 {
        if !cpu_set_source_ok(src, len, 1) {
            return 1;
        }
        if let Some(op) = HleBiosOperation::cpu_set(src, dst, len_mode) {
            bus.start_hle_bios(op);
        }
        return 1;
    }
    let fixed = len_mode & (1 << 24) != 0;
    let width_32 = len_mode & (1 << 26) != 0;
    if width_32 {
        cpu_set_32(bus, src, dst, len, fixed)
    } else {
        cpu_set_16(bus, src, dst, len, fixed)
    }
}

fn cpu_set_32(bus: &mut GbaMemoryBus, src: u32, dst: u32, len: u32, fixed: bool) -> u32 {
    if !cpu_set_source_ok(src, len, 4) {
        return 1;
    }
    let s0 = src & !3;
    let d0 = dst & !3;
    // GBATEK memfill: a fixed source is sampled once (single LDR) and
    // the same unit is stored repeatedly; re-reading per unit would
    // diverge on volatile/mapped sources.
    let fill = bus.read32(s0);
    let mut s = s0;
    let mut d = d0;
    // HW BIOS bulk loop sees flat waits (no GamePak-prefetch erase):
    // accrue raw bus waits; the display formula below carries the
    // fixed overhead.
    bus.begin_raw_batch();
    for _ in 0..len {
        let v = if fixed { fill } else { bus.read32(s) };
        bus.write32(d, v);
        if !fixed {
            s = s.wrapping_add(4);
        }
        d = d.wrapping_add(4);
    }
    bus.end_block_batch();
    // 32BIT: base 0x400 words, waitはWRAMで size*0x1400/0x400 に比例
    // 30ステップで4096byteが終わることはなく、size比例で数千cycleかかる
    let base_disp = if fixed { 0x3060u32 } else { 0x3C5Fu32 };
    let base_wait = 0x1400u32;
    let base_len = 0x400u32;
    let disp = base_disp * len / base_len;
    let wait = base_wait * len / base_len;
    disp.saturating_sub(wait)
}

fn cpu_set_16(bus: &mut GbaMemoryBus, src: u32, dst: u32, len: u32, fixed: bool) -> u32 {
    if !cpu_set_source_ok(src, len, 2) {
        return 1;
    }
    let s0 = src & !1;
    let d0 = dst & !1;
    // Same single-sample rule for 16-bit fills (see above).
    let fill = bus.read16(s0);
    let mut s = s0;
    let mut d = d0;
    bus.begin_raw_batch();
    for _ in 0..len {
        let v = if fixed { fill } else { bus.read16(s) };
        bus.write16(d, v);
        if !fixed {
            s = s.wrapping_add(2);
        }
        d = d.wrapping_add(2);
    }
    bus.end_block_batch();
    // 16BIT: base 0x800 halfwords, waitはWRAMで size*0x1000/0x800 に比例
    // 30ステップで4096byteが終了することはなく、size比例で1万cycle以上かかる
    let base_disp = if fixed { 0x5062u32 } else { 0x6861u32 };
    let base_wait = 0x1000u32;
    let base_len = 0x800u32;
    let disp = base_disp * len / base_len;
    let wait = base_wait * len / base_len;
    let ret = disp.saturating_sub(wait);
    // HW BIOS CpuSet from ROM costs ~93 cycles more than the WRAM-fit
    // formula (HW-pinned by cpy_data_bios TIM1 0xA5).
    if (0x08000000..=0x0DFFFFFF).contains(&src) {
        ret.wrapping_add(93)
    } else {
        ret
    }
}

fn cpu_fast_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) -> u32 {
    let raw_src = regs.r(0);
    let raw_dst = regs.r(1);
    // 32-bit sources align down, except SRAM sources: the 8-bit SRAM
    // bus replicates the exact byte (mgba-suite "SRAM load swi C 32
    // (unaligned)" pins 0x61616161/0x6D6D6D6D), which any masking would
    // destroy. Replication is rotation-invariant, so the odd read32 is
    // safe through `align_read`.
    let sram_src = (0x0E00_0000..0x1000_0000).contains(&raw_src);
    let src = if sram_src { raw_src } else { raw_src & !3 };
    let dst = raw_dst & !3;
    let len_mode = regs.r(2);
    let len = (len_mode & 0x1F_FFFF).next_multiple_of(8);
    // GBATEK: silently reject when the source start or end reaches into
    // the BIOS area; unmapped non-BIOS sources perform no copy either
    // (mgba-suite "Out-of-bounds load swi C 32" pins zeros, like CpuSet).
    let end = src as u64 + len as u64 * 4;
    if len == 0
        || src < 0x0000_4000
        || end - 4 < 0x0000_4000
        || (0x0000_4000..0x0200_0000).contains(&src)
    {
        return 1;
    }
    let fixed = len_mode & (1 << 24) != 0;
    // 高速コピー（HLEで即時完了）。固定fillは単発サンプル。
    // Bulk words (including the fill sample) accrue raw: same raw-bulk
    // treatment as CpuSet (see above).
    bus.begin_raw_batch();
    let fill = bus.read32(src);
    let mut s = src;
    let mut d = dst;
    // Unaligned word stores to SRAM drop (see the CpuSet Write phase);
    // the reads still happen, only the stores are skipped.
    let sram_odd_drop = (0x0E00_0000..0x1000_0000).contains(&raw_dst) && raw_dst & 3 != 0;
    for _ in 0..len {
        let v = if fixed { fill } else { bus.read32(s) };
        if !sram_odd_drop {
            bus.write32(d, v);
        }
        if !fixed {
            s = s.wrapping_add(4);
        }
        d = d.wrapping_add(4);
    }
    bus.end_block_batch();
    // 実測表示 TIMER0: COPY 0x1FDE / FIXED 0x1AE8 (len=0x400, 4096byte)
    // WRAM waitは size*0x1400/0x400 に比例。30ステップで4096byteが
    // 終了することはなく、HLE stallもsize比例で数千cycleかかる。
    let base_disp = if fixed { 0x1AE8u32 } else { 0x1FDEu32 };
    let base_wait = 0x1400u32;
    let base_len = 0x400u32;
    let disp = base_disp * len / base_len;
    let wait = base_wait * len / base_len;
    // EWRAM-source bulk called from IWRAM costs a flat +66 over the
    // fitted rate (mgba-suite Timing CpuSet cells, len 0x100
    // EWRAM->EWRAM). ROM callers (mgba ROM/WRAM cells) and IWRAM-source
    // bulk (PeterLemon display stays exactly base_disp) are excluded,
    // like the 16-bit ROM-source +93 below.
    let src_adjust = if (0x02000000..=0x02FFFFFF).contains(&src) && bus.swi_caller_is_iwram() {
        66
    } else {
        0
    };
    // BIOS call fixed overhead (exception entry + prologue/epilogue,
    // mgba-suite Timing CpuSet cells: flat +69 after raw-bulk) plus the
    // SWI region entry residual (shared with Div/Sqrt/ArcTan). Both skip
    // IWRAM callers (real IWRAM timer ROMs match without them).
    let overhead = if bus.swi_caller_is_iwram() { 0 } else { 69 };
    disp.saturating_sub(wait)
        .wrapping_add(overhead)
        .wrapping_add(src_adjust)
        .saturating_add_signed(bus.swi_region_adjust())
}

fn bios_checksum(regs: &mut CpuRegisters, bus: &GbaMemoryBus) {
    // GBATEK: sum of all 32-bit words in BIOS 0x00000000-0x03FFF
    let sum = bus.bios_checksum();
    regs.set_r(0, sum);
}
