//! Frame validation: dimensions, stride, and tap geometry in one scan.
//! Guards and indexer setup share the single result.

use log::{error, warn};
use nerust_render_traits::{FrameBuffer, PixelFormat};

use super::SoftbufferRenderer;

impl SoftbufferRenderer {
    pub(crate) fn validate_frame(&self, frame: &FrameBuffer) -> Option<ValidatedFrame> {
        if frame.width() != self.render_profile.source_logical_size.width
            || frame.height() != self.render_profile.source_logical_size.height
        {
            error!(
                "Frame size mismatch: expected {}x{}, got {}x{}",
                self.render_profile.source_logical_size.width,
                self.render_profile.source_logical_size.height,
                frame.width(),
                frame.height()
            );
            return None;
        }
        let dst_w = self.size.width as usize;
        let dst_h = self.size.height as usize;
        if dst_w == 0 || dst_h == 0 || self.surface.is_none() {
            return None;
        }
        let avail_h = dst_h.saturating_sub(self.reserved.height_loss(self.size.height) as usize);
        let src_stride = frame.stride();
        let src_h = self.render_profile.source_logical_size.height;
        let src_w = self.render_profile.source_logical_size.width;
        let bpp = frame.format().bytes_per_pixel();
        // Stride must cover one tight row; otherwise the pixel indexer
        // would read past the source on the UI thread.
        if src_stride < src_w.saturating_mul(bpp) {
            warn!(
                "softbuffer stride too small: have {}, need {}",
                src_stride,
                src_w.saturating_mul(bpp)
            );
            return None;
        }
        let use_ntsc = matches!(frame.format(), PixelFormat::PaletteIndex { .. })
            && self.render_profile.ntsc_packed_rgba8.is_some();
        let tap_stride = if matches!(frame.format(), PixelFormat::Rgba) {
            src_stride / 4
        } else if use_ntsc {
            self.render_profile.logical_size.width
        } else {
            src_stride
        };
        if tap_stride == 0 {
            return None;
        }
        let ntsc_writes = if use_ntsc {
            src_h.saturating_mul(self.render_profile.logical_size.width)
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
}

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
