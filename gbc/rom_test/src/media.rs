use std::io::Cursor;

use nerust_render_traits::{FrameBuffer, PixelFormat};
use png::{BitDepth, ColorType, Encoder};

use super::error::RomTestError;

/// Create a FrameBuffer suitable for GBC screen capture (160×144 RGBA).
pub fn screen_buffer() -> FrameBuffer {
    FrameBuffer::with_capacity(160, 144, PixelFormat::Rgba)
}

/// Encode a FrameBuffer as PNG bytes, handling stride padding.
pub fn encode_screenshot_png(fb: &FrameBuffer) -> Result<Vec<u8>, RomTestError> {
    let w = fb.width();
    let h = fb.height();
    let stride = fb.stride();
    let src = fb.as_ref();

    // Handle stride: if stride > w * 4, skip padding bytes
    let rgba = if stride == w * 4 {
        src.to_vec()
    } else {
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let row_start = y * stride;
            rgba.extend_from_slice(&src[row_start..row_start + w * 4]);
        }
        rgba
    };

    let mut encoded = Cursor::new(Vec::new());
    let mut encoder = Encoder::new(&mut encoded, w as u32, h as u32);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgba)?;
    drop(writer);

    Ok(encoded.into_inner())
}
