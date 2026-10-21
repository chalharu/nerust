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
        let rgba = layer.rgba();
        let need = src_w.checked_mul(src_h).and_then(|px| px.checked_mul(4));
        let Some(need) = need else { return };
        if src_w == 0 || src_h == 0 || rgba.len() < need {
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
                let alpha = rgba[s + 3];
                if alpha == 0 {
                    continue;
                }
                // Softbuffer presents XRGB: the high byte is not alpha. The
                // game/clear pixels are an opaque destination for source-over.
                let Some(dst_word) = buffer.get_mut(dst_y * dst_w + dst_x) else {
                    continue;
                };
                let dst_word_value = *dst_word | 0xFF00_0000;
                let dst = RgbaWord::from_argb_word(dst_word_value);
                let src = [rgba[s], rgba[s + 1], rgba[s + 2], alpha];
                *dst_word = dst.blend_src_over(src).as_argb_word();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SoftbufferRenderer;
    use nerust_render_traits::renderer::OsdLayer;

    #[test]
    fn right_clipping_keeps_original_row_pitch() {
        let mut buffer = [0u32; 4];
        let layer = OsdLayer::new(
            1,
            0,
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        );
        SoftbufferRenderer::blend_osd(&mut buffer, 2, 2, &layer);
        assert_eq!(buffer, [0, 0xFFFF_0000, 0, 0xFF00_00FF]);
    }

    fn layer_at(x: u32, y: u32, rgba: Vec<u8>) -> OsdLayer {
        OsdLayer::new(x, y, 2, 2, rgba)
    }

    #[test]
    fn clips_layer_outside_buffer() {
        let mut buffer = vec![0xFF00_0000u32; 4];
        let before = buffer.clone();
        let layer = layer_at(9, 9, vec![255u8; 16]);
        SoftbufferRenderer::blend_osd(&mut buffer, 2, 2, &layer);
        assert_eq!(buffer, before);
    }

    #[test]
    fn short_rgba_and_empty_layer_are_noops() {
        let mut buffer = vec![0xFF00_0000u32; 4];
        let before = buffer.clone();
        let layer = layer_at(0, 0, vec![255u8; 3]);
        SoftbufferRenderer::blend_osd(&mut buffer, 2, 2, &layer);
        assert_eq!(buffer, before);
        let empty = OsdLayer::new(0, 0, 0, 0, vec![]);
        SoftbufferRenderer::blend_osd(&mut buffer, 2, 2, &empty);
        assert_eq!(buffer, before);
    }

    #[test]
    fn transparent_pixels_keep_dst() {
        let mut buffer = vec![0xFF00_0000u32; 4];
        let layer = layer_at(0, 0, vec![0u8; 16]);
        SoftbufferRenderer::blend_osd(&mut buffer, 2, 2, &layer);
        assert_eq!(buffer, [0xFF00_0000u32; 4]);
        let mut partial = vec![0xFF00_0000u32; 4];
        let layer = layer_at(
            1,
            1,
            vec![255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        SoftbufferRenderer::blend_osd(&mut partial, 2, 2, &layer);
        assert_eq!(partial[3], 0xFFFF_0000);
        assert_eq!(&partial[..3], &[0xFF00_0000u32; 3]);
    }
}
