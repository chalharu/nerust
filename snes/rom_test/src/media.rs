use nerust_crc64_hasher::Crc64Hasher;
use png::{BitDepth, ColorType, Decoder, Encoder};
use std::fs;
use std::hash::Hasher;
use std::io::Cursor;
use std::path::Path;

pub const SCREEN_WIDTH: usize = 256;
pub const SCREEN_HEIGHT: usize = 224;

pub fn screen_hash_rgba(rgba: &[u8]) -> u64 {
    let mut hasher = Crc64Hasher::new();
    hasher.write(rgba);
    hasher.finish()
}

pub fn load_png_rgba(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::read(path).map_err(|e| format!("failed to read `{}`: {e}", path.display()))?;
    let cursor = Cursor::new(file);
    let decoder = Decoder::new(cursor);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("failed to decode `{}`: {e}", path.display()))?;
    let width = reader.info().width as usize;
    let height = reader.info().height as usize;
    let pixel_count = width * height;
    let buf_size = reader.output_buffer_size().unwrap_or(pixel_count * 4);
    let mut raw = vec![0u8; buf_size];
    let _info = reader
        .next_frame(&mut raw)
        .map_err(|e| format!("failed to read `{}`: {e}", path.display()))?;

    let output_size = pixel_count * 4;
    let rgba = if raw.len() >= output_size {
        raw[..output_size].to_vec()
    } else {
        // Paletted PNG with bit_depth < 8: pixels are packed (e.g. 4-bit = 2 pixels/byte).
        // The decoder provides raw packed bytes; expand to RGBA via palette lookup.
        let bits_per_pixel = match reader.info().bit_depth {
            png::BitDepth::One => 1,
            png::BitDepth::Two => 2,
            png::BitDepth::Four => 4,
            png::BitDepth::Eight => 8,
            png::BitDepth::Sixteen => 16,
        };
        let pixels_per_byte = if bits_per_pixel < 8 {
            8 / bits_per_pixel
        } else {
            1
        };
        let palette = reader.info().palette.as_ref();
        let trns = reader.info().trns.as_ref();
        let mut rgba = Vec::with_capacity(output_size);
        for i in 0..pixel_count {
            let (r, g, b, a) = if pixels_per_byte > 1
                && let Some(pal) = palette
            {
                // Packed indexed pixels: extract nibble/bit and look up palette.
                let byte_idx = i / pixels_per_byte;
                let shift = ((i % pixels_per_byte) * bits_per_pixel) as u8;
                let idx = if byte_idx < raw.len() {
                    (raw[byte_idx] >> shift) & ((1u8 << bits_per_pixel) - 1)
                } else {
                    0
                } as usize;
                if idx * 3 + 2 < pal.len() {
                    let alpha = trns.and_then(|t| t.get(idx).copied()).unwrap_or(0xFF);
                    (pal[idx * 3], pal[idx * 3 + 1], pal[idx * 3 + 2], alpha)
                } else {
                    (0, 0, 0, 0xFF)
                }
            } else {
                // Full-byte format (RGB, RGBA, or grayscale).
                let src_bpp = if raw.len() >= pixel_count * 3 { 3 } else { 1 };
                let src = i * src_bpp;
                let r = if src < raw.len() { raw[src] } else { 0 };
                let g = if src_bpp > 1 && src + 1 < raw.len() {
                    raw[src + 1]
                } else {
                    r
                };
                let b = if src_bpp > 2 && src + 2 < raw.len() {
                    raw[src + 2]
                } else {
                    r
                };
                (r, g, b, 0xFF)
            };
            rgba.push(r);
            rgba.push(g);
            rgba.push(b);
            rgba.push(a);
        }
        rgba
    };
    Ok(rgba)
}

pub fn png_hash_from_path(path: &Path) -> Result<u64, String> {
    let rgba = load_png_rgba(path)?;
    let mut hasher = Crc64Hasher::new();
    hasher.write(&rgba);
    Ok(hasher.finish())
}

pub fn encode_screenshot_png(
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, png::EncodingError> {
    let mut encoded = Cursor::new(Vec::new());
    let mut encoder = Encoder::new(&mut encoded, width, height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    drop(writer);
    Ok(encoded.into_inner())
}

#[cfg(test)]
mod tests {
    use super::screen_hash_rgba;

    #[test]
    fn screen_hash_changes_with_pixel_content() {
        let first = [0x00, 0x10, 0x20, 0xFF, 0x40, 0x50, 0x60, 0xFF];
        let second = [0x00, 0x10, 0x20, 0xFF, 0x40, 0x50, 0x61, 0xFF];

        assert_eq!(screen_hash_rgba(&first), screen_hash_rgba(&first));
        assert_ne!(screen_hash_rgba(&first), screen_hash_rgba(&second));
    }
}
