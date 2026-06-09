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

pub fn png_hash_from_path(path: &Path) -> Result<u64, String> {
    let file = fs::read(path).map_err(|e| format!("failed to read `{}`: {e}", path.display()))?;
    let cursor = Cursor::new(file);
    let decoder = Decoder::new(cursor);
    let mut reader = decoder.read_info().map_err(|e| format!("failed to decode `{}`: {e}", path.display()))?;
    let width = reader.info().width as usize;
    let height = reader.info().height as usize;
    let pixel_count = width * height;
    let buf_size = reader.output_buffer_size().unwrap_or(pixel_count * 4);
    let mut raw = vec![0u8; buf_size];
    let _info = reader.next_frame(&mut raw).map_err(|e| format!("failed to read `{}`: {e}", path.display()))?;

    let output_size = pixel_count * 4;
    let rgba = if raw.len() >= output_size {
        raw[..output_size].to_vec()
    } else {
        let mut rgba = Vec::with_capacity(output_size);
        let src_bpp = if raw.len() >= pixel_count * 3 { 3 } else { 1 };
        for i in 0..pixel_count {
            let src = i * src_bpp;
            rgba.push(if src_bpp > 0 { raw[src] } else { 0 });
            rgba.push(if src_bpp > 1 { raw[src + 1] } else { 0 });
            rgba.push(if src_bpp > 2 { raw[src + 2] } else { 0 });
            rgba.push(0xFF);
        }
        rgba
    };

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
