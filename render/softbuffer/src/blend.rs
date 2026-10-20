//! OSD blend: straight-alpha composite of the layer, clipped to the buffer.
//! Integer fixed point with round-half-up; no FPU per pixel.

use super::SoftbufferRenderer;

impl SoftbufferRenderer {
    /// Straight-alpha blend of the OSD layer, clipped to the buffer.
    pub(crate) fn blend_osd(
        buffer: &mut [u32],
        dst_w: usize,
        dst_h: usize,
        layer: &nerust_render_traits::renderer::OsdLayer,
    ) {
        let src_w = layer.width as usize;
        let src_h = layer.height as usize;
        let need = src_w.checked_mul(src_h).and_then(|px| px.checked_mul(4));
        let Some(need) = need else { return };
        if src_w == 0 || src_h == 0 || layer.rgba.len() < need {
            return;
        }
        for row in 0..src_h {
            let dst_y = layer.y as usize + row;
            if dst_y >= dst_h {
                break;
            }
            for col in 0..src_w {
                let dst_x = layer.x as usize + col;
                if dst_x >= dst_w {
                    break;
                }
                let s = (row * src_w + col) * 4;
                let alpha = u32::from(layer.rgba[s + 3]);
                if alpha == 0 {
                    continue;
                }
                // Buffer words are 0xAARRGGBB. Shift out channels
                // directly instead of `to_be_bytes` round-trips.
                let dst_word = buffer[dst_y * dst_w + dst_x];
                let (dr, dg, db) = (
                    ((dst_word >> 16) & 0xff) as u8,
                    ((dst_word >> 8) & 0xff) as u8,
                    (dst_word & 0xff) as u8,
                );
                let blend = |src: u8, dst: u8| {
                    ((u32::from(src) * alpha + u32::from(dst) * (255 - alpha) + 127) / 255) as u8
                };
                buffer[dst_y * dst_w + dst_x] = 0xff00_0000
                    | (u32::from(blend(layer.rgba[s], dr)) << 16)
                    | (u32::from(blend(layer.rgba[s + 1], dg)) << 8)
                    | u32::from(blend(layer.rgba[s + 2], db));
            }
        }
    }
}
