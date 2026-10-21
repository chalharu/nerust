//! Frame validation: dimensions, stride, and tap geometry in one scan.
//! Guards and indexer setup share the single result.

use log::warn;
use nerust_render_traits::{FrameBuffer, FrameLayout, PixelFormat};

use super::SoftbufferRenderer;

/// Validated frame geometry shared by the blit path. Computed once, so the
/// guards and indexer setup share a single scan.
pub(crate) struct ValidatedFrame {
    pub(crate) dst_w: usize,
    pub(crate) dst_h: usize,
    pub(crate) avail_h: usize,
    pub(crate) src_stride: usize,
    pub(crate) src_h: usize,
    pub(crate) tap_stride: usize,
    pub(crate) ntsc_writes: usize,
}

impl SoftbufferRenderer {
    pub(crate) fn validate_frame(&self, frame: &FrameBuffer) -> Option<ValidatedFrame> {
        let input = Self::validate_input(self.profile(), frame)?;
        let dst_w = self.output_size().width as usize;
        let dst_h = self.output_size().height as usize;
        if dst_w == 0 || dst_h == 0 || !self.surface_attached() {
            return None;
        }
        let avail_h = dst_h
            .saturating_sub(self.reserved_strip().height_loss(self.output_size().height) as usize);
        let src_stride = frame.stride();
        let use_ntsc = matches!(frame.format(), PixelFormat::PaletteIndex { .. })
            && self.profile().ntsc_packed_rgba8.is_some();
        let src_h = if use_ntsc {
            self.profile().logical_size.height
        } else {
            input.size().height
        };
        let tap_stride = if matches!(frame.format(), PixelFormat::Rgba) {
            src_stride / 4
        } else if use_ntsc {
            self.profile().logical_size.width
        } else {
            src_stride
        };
        if tap_stride == 0 {
            return None;
        }
        let ntsc_writes = if use_ntsc {
            src_h.saturating_mul(self.profile().logical_size.width)
        } else {
            0
        };
        Some(ValidatedFrame {
            dst_w,
            dst_h,
            avail_h,
            src_stride,
            src_h,
            tap_stride,
            ntsc_writes,
        })
    }

    /// Pure input gate, also usable in headless tests without a native surface.
    pub(crate) fn validate_input(
        profile: &nerust_render_traits::VideoRenderProfile,
        frame: &FrameBuffer,
    ) -> Option<FrameLayout> {
        FrameLayout::validate(frame, profile.input_size(), profile.frame_format)
            .map_err(|error| warn!("softbuffer: {error}"))
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_render_traits::{
        VideoFrameFormat, VideoRenderProfile, logical::LogicalSize, physical::PhysicalSize,
    };

    fn profile(format: VideoFrameFormat) -> VideoRenderProfile {
        VideoRenderProfile {
            source_logical_size: LogicalSize {
                width: 2,
                height: 2,
            },
            logical_size: LogicalSize {
                width: 4,
                height: 4,
            },
            physical_size: PhysicalSize {
                width: 4.0,
                height: 4.0,
            },
            frame_format: format,
            ntsc_packed_rgba8: None,
        }
    }

    #[test]
    fn filtered_rgba_and_source_palette_use_distinct_contracts() {
        let rgba = profile(VideoFrameFormat::Rgba);
        let palette = profile(VideoFrameFormat::Palette);
        let mut frame = FrameBuffer::with_capacity(4, 4, PixelFormat::Rgba);
        frame.resize(4, 4);
        assert!(SoftbufferRenderer::validate_input(&rgba, &frame).is_some());
        assert!(SoftbufferRenderer::validate_input(&palette, &frame).is_none());
        frame.resize_data(3 * frame.stride() + 16);
        assert!(SoftbufferRenderer::validate_input(&rgba, &frame).is_some());
        frame.resize_data(3 * frame.stride() + 15);
        assert!(SoftbufferRenderer::validate_input(&rgba, &frame).is_none());
        frame.resize(2, 2);
        assert!(SoftbufferRenderer::validate_input(&rgba, &frame).is_none());
        frame.set_format(PixelFormat::PaletteIndex {
            palette: vec![0; 64].into_boxed_slice(),
        });
        let valid = SoftbufferRenderer::validate_input(&palette, &frame).unwrap();
        assert_eq!(valid.size(), palette.source_logical_size);
        // A non-NTSC palette profile resizes from the original image, not the filtered dimensions.
        let mut renderer = SoftbufferRenderer::new(&palette, 3);
        renderer.size = nerust_render_traits::SurfaceSize::new(8, 8);
        renderer.resize_lut();
        assert_eq!(renderer.resize_buffer.len(), 8 * 2);
        assert!(renderer.lut.y_lut.iter().flatten().all(|&(i, _)| i < 2));
    }
}
