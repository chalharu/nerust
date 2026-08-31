use std::io::Cursor;

use nerust_render_traits::FrameBuffer;
use png::{BitDepth, ColorType, Encoder};

pub fn encode_screenshot_png(fb: &FrameBuffer) -> Result<Vec<u8>, png::EncodingError> {
    let mut rgba = Vec::with_capacity(fb.width() * fb.height() * 4);
    for y in 0..fb.height() {
        let start = y * fb.stride();
        rgba.extend_from_slice(&fb.as_ref()[start..start + fb.width() * 4]);
    }
    let mut encoded = Cursor::new(Vec::new());
    let mut encoder = Encoder::new(&mut encoded, fb.width() as u32, fb.height() as u32);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgba)?;
    drop(writer);
    Ok(encoded.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_render_traits::PixelFormat;

    #[test]
    fn encodes_padded_rgba_framebuffer() {
        let mut framebuffer = FrameBuffer::with_capacity(3, 2, PixelFormat::Rgba);
        framebuffer.resize(3, 2);
        for row in 0..2 {
            let start = row * framebuffer.stride();
            framebuffer.as_mut()[start..start + 12].fill((row + 1) as u8);
        }
        let png = encode_screenshot_png(&framebuffer).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() > 16);
    }
}
