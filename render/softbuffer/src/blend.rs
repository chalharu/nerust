//! OSD blend: straight-alpha composite of the layer, clipped to the buffer.
//! Whole-word load/store through [`RgbaWord`]; per-channel math is hoisted once.

use nerust_render_traits::rgb::RgbaWord;

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
                let alpha = layer.rgba[s + 3];
                if alpha == 0 {
                    continue;
                }
                let dst_word = buffer[dst_y * dst_w + dst_x];
                let dst = RgbaWord::from_argb_word(dst_word);
                let src = [layer.rgba[s], layer.rgba[s + 1], layer.rgba[s + 2], alpha];
                buffer[dst_y * dst_w + dst_x] = dst.blend_src_over(src).as_argb_word();
            }
        }
    }
}
