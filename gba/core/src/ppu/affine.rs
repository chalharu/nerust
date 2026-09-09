//! PPU affine helpers — thin wrapper over `crate::math::affine` shared with BIOS HLE.
//!
//! `architecture-v1.md` expects `ppu/affine.rs` as part of the PPU crate budget.
//! The actual matrix math lives in `crate::math::affine` (Phase 6/8 shared) to avoid
//! duplication with `bios::handle_swi` `BgAffineSet`/`ObjAffineSet`. This module
//! re-exports the shared types and provides PPU-specific accumulator helpers.

/// Advance the internal affine accumulators by one scanline (PB/PD),
/// skipping the increment when a HBlank BGX/Y write has already updated
/// the reference for the next line (GBATEK per-scanline affine), and
/// skipping disabled BGs entirely (NBA #177).
#[inline]
pub fn advance_line(
    internal_x: &mut [i32; 2],
    internal_y: &mut [i32; 2],
    pb: [i16; 2],
    pd: [i16; 2],
    ref_written: &mut [bool; 2],
    enabled: [bool; 2],
) {
    for affine in 0..2 {
        if !enabled[affine] {
            ref_written[affine] = false;
            continue;
        }
        if ref_written[affine] {
            ref_written[affine] = false;
            continue;
        }
        internal_x[affine] = internal_x[affine].wrapping_add(i32::from(pb[affine]));
        internal_y[affine] = internal_y[affine].wrapping_add(i32::from(pd[affine]));
    }
}
