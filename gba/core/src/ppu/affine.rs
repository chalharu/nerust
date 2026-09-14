//! PPU affine helpers over the shared affine math (also used by BIOS HLE).
//! Provides PPU-specific internal-reference accumulator stepping.

/// Advance the internal affine accumulators by one scanline (PB/PD),
/// skipping disabled BGs entirely (HW-confirmed).
#[inline]
pub fn advance_line(
    internal_x: &mut [i32; 2],
    internal_y: &mut [i32; 2],
    pb: [i16; 2],
    pd: [i16; 2],
    enabled: [bool; 2],
) {
    for affine in 0..2 {
        if !enabled[affine] {
            continue;
        }
        internal_x[affine] = internal_x[affine].wrapping_add(i32::from(pb[affine]));
        internal_y[affine] = internal_y[affine].wrapping_add(i32::from(pd[affine]));
    }
}
