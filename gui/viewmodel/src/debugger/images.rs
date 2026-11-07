//! System image blit and scaling. Pure; frontends share these
//! so the PPU viewer shows identical pixels everywhere.

pub use nerust_gui_runtime::debugger::{DebugImageRgba, rgba_image};

/// PPU viewer empty-hover hint. Canonical wording for every
/// frontend; PPU image titles render the raw kernel label.
pub const PPU_HOVER_HINT: &str = "hover a tile";

/// Tile hover geometry: 2x display scale, 8x8 tiles, byte offset of the
/// tile start (+16 per tile). Generic geometry, no per-system branch.
/// Frontends format the result with `tile_hover`; the math lives here
/// once so both viewers agree pixel-for-pixel.
pub fn tile_hover_geometry(width: u32, height: u32, x: f64, y: f64) -> (u32, u32, u32) {
    let sx = (x / 2.0).floor().clamp(0.0, f64::from(width) - 1.0) as u32;
    let sy = (y / 2.0).floor().clamp(0.0, f64::from(height) - 1.0) as u32;
    let cols = (width / 8).max(1);
    let idx = (sy / 8) * cols + (sx / 8);
    (idx, sx, sy)
}

/// Tile hover line: 2x display scale, 8x8 tiles, byte offset of the
/// tile start (+16 per tile). Shared by gtk/tao PPU viewers so both
/// agree pixel-for-pixel.
pub fn tile_hover_text(
    language: nerust_settings_core::model::language::AppLanguage,
    label: &str,
    width: u32,
    height: u32,
    x: f64,
    y: f64,
) -> String {
    let (idx, sx, sy) = tile_hover_geometry(width, height, x, y);
    nerust_settings_core::i18n::tile_hover(language, label, idx, sx, sy, idx * 16)
}

/// Nearest-neighbor 2x scale of blitted RGBA bytes. Pure; frontends
/// share it so the PPU viewer shows identical pixels everywhere.
pub fn scale2x_nearest(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut out = vec![0u8; w * 2 * h * 2 * 4];
    for y in 0..h {
        for x in 0..w {
            let src = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
            for dy in 0..2 {
                for dx in 0..2 {
                    let dst = ((y * 2 + dy) * w * 2 + (x * 2 + dx)) * 4;
                    out[dst..dst + 4].copy_from_slice(src);
                }
            }
        }
    }
    out
}
