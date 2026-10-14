//! System image blit and scaling. Pure; frontends share these
//! so the PPU viewer shows identical pixels everywhere.

use nerust_core_traits::debugger::{DebugImage, ImageFormat};

/// PPU viewer empty-hover hint. Canonical wording for every
/// frontend; PPU image titles render the raw kernel label.
pub const PPU_HOVER_HINT: &str = "hover a tile";

/// Blit an indexed system image to `(label, width, height, rgba)`.
/// The blit is mechanical (no system knowledge): indexed pixels
/// through the descriptor palette, opaque alpha.
pub fn rgba_image(image: &DebugImage) -> (String, u32, u32, Vec<u8>) {
    let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
    for &pixel in &image.pixels {
        let rgb = match image.format {
            ImageFormat::Indexed2bpp | ImageFormat::Indexed4bpp => image
                .palette
                .get(pixel as usize)
                .copied()
                .unwrap_or([0, 0, 0]),
        };
        rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 0xFF]);
    }
    (image.label_id.to_string(), image.width, image.height, rgba)
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
