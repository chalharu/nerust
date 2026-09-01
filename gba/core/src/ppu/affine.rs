//! PPU affine helpers — thin wrapper over `crate::math::affine` shared with BIOS HLE.
//!
//! `architecture-v1.md` expects `ppu/affine.rs` as part of the PPU crate budget.
//! The actual matrix math lives in `crate::math::affine` (Phase 6/8 shared) to avoid
//! duplication with `bios::handle_swi` `BgAffineSet`/`ObjAffineSet`. This module
//! re-exports the shared types and provides PPU-specific accumulator helpers.

/// Advance the internal affine accumulators by one scanline (PB/PD).
#[inline]
pub fn advance_line(
    internal_x: &mut [i32; 2],
    internal_y: &mut [i32; 2],
    pb: [i16; 2],
    pd: [i16; 2],
) {
    for affine in 0..2 {
        internal_x[affine] = internal_x[affine].wrapping_add(i32::from(pb[affine]));
        internal_y[affine] = internal_y[affine].wrapping_add(i32::from(pd[affine]));
    }
}
