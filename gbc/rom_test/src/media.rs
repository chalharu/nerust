use std::io::Cursor;

use nerust_render_traits::FrameBuffer;
use png::{BitDepth, ColorType, Decoder, Encoder, Transformations};

use super::error::RomTestError;

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

    encode_rgba_png(w as u32, h as u32, &rgba)
}

/// Encode raw RGBA pixels as PNG bytes.
pub fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, RomTestError> {
    let mut encoded = Cursor::new(Vec::new());
    let mut encoder = Encoder::new(&mut encoded, width, height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    drop(writer);
    Ok(encoded.into_inner())
}

/// Decode a PNG into 8-bit RGB pixels, expanding palettes and grayscale.
pub fn decode_png_rgb(data: &[u8]) -> Result<(u32, u32, Vec<u8>), RomTestError> {
    let mut decoder = Decoder::new(Cursor::new(data));
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());

    let w = info.width as usize;
    let h = info.height as usize;
    let pixels = w * h;
    let rgb = match buf.len() {
        // Grayscale (1 byte per pixel): expand to RGB.
        n if n == pixels => {
            let mut out = Vec::with_capacity(pixels * 3);
            for &v in &buf {
                out.extend_from_slice(&[v, v, v]);
            }
            out
        }
        // Grayscale + alpha: drop alpha.
        n if n == pixels * 2 => {
            let mut out = Vec::with_capacity(pixels * 3);
            for px in buf.chunks_exact(2) {
                out.extend_from_slice(&[px[0], px[0], px[0]]);
            }
            out
        }
        // RGB or RGBA (drop alpha).
        n if n == pixels * 4 => buf
            .chunks_exact(4)
            .flat_map(|px| px[..3].to_vec())
            .collect(),
        _ => buf,
    };
    Ok((info.width, info.height, rgb))
}

/// Compose a side-by-side comparison image: actual | reference | diff
/// (red where pixels differ). Output is RGBA.
pub fn compose_diff_image(actual: &[u8], reference: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 3 * 4) as usize);
    let size = (w * h) as usize;
    for i in 0..size {
        let a = &actual[i * 3..i * 3 + 3];
        let r = &reference[i * 3..i * 3 + 3];
        let diff = if a == r { a } else { &[255, 0, 0] };
        for px in [a, r, diff] {
            out.extend_from_slice(px);
            out.push(255);
        }
    }
    out
}
