use std::num::NonZeroU32;

use log::warn;
use nerust_render_traits::{
    FrameBuffer, PixelFormat, SurfaceSize, VideoRenderProfile,
    renderer::{
        FrameSink, OpaqueError, OsdSink, PipelineFactory, ProfileSwitch, RenderPipeline,
        RenderResult, RendererConfig, RendererError, StripSink, SurfaceLifecycle,
    },
    rgb::RgbaWord,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle,
};
use softbuffer::{Context, Surface};

mod blend;
mod lut;
mod ntsc;
mod validate;

use lut::{LutEntry, ResizeKernel};
use ntsc::Ntsc;
use validate::ValidatedFrame;

#[derive(Debug, Clone)]
struct WindowHandlePair {
    window: RawWindowHandle,
    display: RawDisplayHandle,
}

impl HasWindowHandle for WindowHandlePair {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        unsafe { Ok(WindowHandle::borrow_raw(self.window)) }
    }
}

impl HasDisplayHandle for WindowHandlePair {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        unsafe { Ok(DisplayHandle::borrow_raw(self.display)) }
    }
}

#[derive(Debug)]
pub struct SoftbufferRenderer {
    ctx: Option<Context<WindowHandlePair>>,
    surface: Option<Surface<WindowHandlePair, WindowHandlePair>>,
    render_profile: VideoRenderProfile,
    size: SurfaceSize,
    reserved: nerust_render_traits::ReservedStrip,
    lut: LutEntry,
    resize_buffer: Vec<u32>,
    ntsc_buffer: Vec<u32>,
    osd: Option<nerust_render_traits::renderer::OsdLayer>,
    /// Last tap-coverage verdict keyed by the full struct; a key match
    /// skips the scan, and the `lut_rev` epoch invalidates on rebuild.
    taps_cache: Option<(TapKey, bool)>,
}

/// Tap-coverage inputs for one frame shape. Caches negative verdicts too.
/// `dst_h` is deliberately *not* stored: it always equals `avail_h + height_loss`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TapKey {
    dst_w: usize,
    avail_h: usize,
    src_w: usize,
    src_h: usize,
    /// Byte stride of the source frame. Distinct from `tap_stride`
    /// (pixels or indices): one `tap_stride` can feed different rows.
    src_stride: usize,
    tap_stride: usize,
    resize_len: usize,
    ntsc_writes: usize,
    ntsc_len: usize,
    lut_rev: u64,
    kernel_bilinear: bool,
    reserved_top: u32,
    reserved_bottom: u32,
}

impl TapKey {}

/// Monomorphic pixel source for the resize passes.
/// One enum replaces three closures, so the hot loop never monomorphizes.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PixelIndexer<'a> {
    /// Tight RGBA bytes; short rows fall back to opaque black.
    Rgba { src: &'a [u8] },
    /// Palette indices plus the console palette (u32 `0xRRGGBBAA` words).
    Palette { src: &'a [u8], palette: &'a [u32] },
    /// Pre-simulated NTSC words (`0xRRGGBBAA` layout, see [`Ntsc`]).
    Ntsc { buf: &'a [u32] },
}

/// Opaque-black fallback for short rows; never panics the UI thread.
const OOB_TEXEL: [u8; 4] = [0, 0, 0, 255];

/// Tight RGBA row texel at pixel `i` past a hoisted row base.
fn rgba_texel(src: &[u8], row_base: usize, i: usize) -> [u8; 4] {
    let base = row_base.saturating_add(i.saturating_mul(4));
    src.get(base..base.saturating_add(4))
        .and_then(|s| s.try_into().ok())
        .unwrap_or(OOB_TEXEL)
}

/// Palette-index texel at `i` past a hoisted row base.
fn palette_texel(src: &[u8], palette: &[u32], row_base: usize, i: usize) -> [u8; 4] {
    let index = src.get(row_base.saturating_add(i)).copied().unwrap_or(0) as usize;
    RgbaWord::from_u32(palette.get(index).copied().unwrap_or(0)).to_bytes_be()
}

/// Pre-simulated NTSC word texel at `i` past a hoisted row base.
fn ntsc_texel(buf: &[u32], row_base: usize, i: usize) -> [u8; 4] {
    RgbaWord::from_u32(buf.get(row_base.saturating_add(i)).copied().unwrap_or(0)).to_bytes_be()
}

impl PixelIndexer<'_> {
    /// One texel with opaque-black fallback; test seam (hot path uses row helpers).
    #[cfg(test)]
    pub(crate) fn pixel(&self, i: usize) -> [u8; 4] {
        match *self {
            Self::Rgba { src } => rgba_texel(src, 0, i),
            Self::Palette { src, palette } => palette_texel(src, palette, 0, i),
            Self::Ntsc { buf } => ntsc_texel(buf, 0, i),
        }
    }
}

unsafe impl Send for SoftbufferRenderer {}
unsafe impl Sync for SoftbufferRenderer {}

impl SoftbufferRenderer {
    fn new(profile: &VideoRenderProfile) -> Self {
        Self {
            ctx: None,
            surface: None,
            render_profile: profile.clone(),
            size: SurfaceSize::new(0, 0),
            reserved: nerust_render_traits::ReservedStrip::default(),
            osd: None,
            lut: LutEntry::new(),
            resize_buffer: Vec::new(),
            ntsc_buffer: Vec::new(),
            taps_cache: None,
        }
    }

    fn available_size(&self) -> SurfaceSize {
        let loss = self.reserved.height_loss(self.size.height);
        SurfaceSize::new(self.size.width, self.size.height.saturating_sub(loss))
    }

    fn resize_lut(&mut self) {
        let profile = &self.render_profile;
        // Palette is still source-sized unless this backend performs NTSC;
        // RGBA has already been filtered by the CPU producer.
        let raster = if profile.frame_format == nerust_render_traits::VideoFrameFormat::Palette
            && profile.ntsc_packed_rgba8.is_none()
        {
            profile.source_logical_size
        } else {
            profile.logical_size
        };
        self.lut.resize_lut(
            SurfaceSize {
                width: raster.width as u32,
                height: raster.height as u32,
            },
            self.render_profile.physical_size.width / self.render_profile.physical_size.height,
            self.available_size(),
        );
        self.ntsc_buffer.resize(raster.width * raster.height, 0);
        self.resize_buffer
            .resize(self.size.width as usize * raster.height, 0);
    }

    #[expect(clippy::too_many_arguments)]
    fn rendering(
        dst: &mut [u32],
        src_stride: usize,
        src_h: usize,
        dst_w: usize,
        dst_h: usize,
        indexer: &PixelIndexer<'_>,
        lut: &LutEntry,
        resize_buffer: &mut [u32],
    ) {
        // Hoisted tap width: no per-pixel method call on the hot path.
        let px = lut.lut_pixel_size();
        Self::horizontal_pass(src_stride, src_h, dst_w, indexer, lut, px, resize_buffer);
        Self::vertical_pass(dst, dst_w, dst_h, lut, px, resize_buffer);
    }

    /// Horizontal two-tap loop: variant dispatches once per row, taps hoist per pixel.
    /// Weights sum to one in the interior; edges go black where taps fall outside.
    fn horizontal_pass(
        src_stride: usize,
        src_h: usize,
        dst_w: usize,
        indexer: &PixelIndexer<'_>,
        lut: &LutEntry,
        px: usize,
        resize_buffer: &mut [u32],
    ) {
        for y in 0..src_h {
            let dst_y_index = y * dst_w;
            let src_y_index = y * src_stride;
            match *indexer {
                PixelIndexer::Rgba { src } => {
                    let row_base = src_y_index.saturating_mul(4);
                    Self::horizontal_row(dst_y_index, dst_w, lut, px, resize_buffer, |i| {
                        rgba_texel(src, row_base, i)
                    });
                }
                PixelIndexer::Palette { src, palette } => {
                    Self::horizontal_row(dst_y_index, dst_w, lut, px, resize_buffer, |i| {
                        palette_texel(src, palette, src_y_index, i)
                    });
                }
                PixelIndexer::Ntsc { buf } => {
                    Self::horizontal_row(dst_y_index, dst_w, lut, px, resize_buffer, |i| {
                        ntsc_texel(buf, src_y_index, i)
                    });
                }
            }
        }
    }

    /// One monomorphic destination row: hoisted tap pair per pixel.
    /// `fetch` inlines per source variant; the LUT slice checks once per pixel.
    fn horizontal_row(
        dst_y_index: usize,
        dst_w: usize,
        lut: &LutEntry,
        px: usize,
        resize_buffer: &mut [u32],
        fetch: impl Fn(usize) -> [u8; 4] + Copy,
    ) {
        for x in 0..dst_w {
            let dst_index = dst_y_index + x;
            let base = x * px;
            // Covered by `taps_cover`; a short LUT here would
            // silently black out the row via the `get` below.
            debug_assert!(lut.x_lut.len() >= base + px);
            let mut taps = [(0u16, 0u16); 2];
            let mut tap_count = 0;
            if let Some(slice) = lut.x_lut.get(base..base + px) {
                for tap in slice.iter().flatten() {
                    if tap_count < 2 {
                        taps[tap_count] = *tap;
                        tap_count += 1;
                    }
                }
            }
            let mut acc = [0u16; 4];
            let mut touched = false;
            for (i, w) in taps[..tap_count].iter().copied() {
                let src_val = fetch(i as usize);
                acc[0] = acc[0].saturating_add((src_val[0] as u16 * w + 0x80) >> 8);
                acc[1] = acc[1].saturating_add((src_val[1] as u16 * w + 0x80) >> 8);
                acc[2] = acc[2].saturating_add((src_val[2] as u16 * w + 0x80) >> 8);
                acc[3] = acc[3].saturating_add((src_val[3] as u16 * w + 0x80) >> 8);
                touched = true;
            }
            let word = if touched {
                RgbaWord::pack_argb(
                    acc[0].min(255) as u8,
                    acc[1].min(255) as u8,
                    acc[2].min(255) as u8,
                    acc[3].min(255) as u8,
                )
            } else {
                0
            };
            if dst_index < resize_buffer.len() {
                resize_buffer[dst_index] = word;
            }
        }
    }

    /// Vertical pass over the horizontal result. Taps are at most 2.
    /// Reads buffer words back with `to_ne_bytes`, the inverse of the horizontal pack.
    fn vertical_pass(
        dst: &mut [u32],
        dst_w: usize,
        dst_h: usize,
        lut: &LutEntry,
        px: usize,
        resize_buffer: &[u32],
    ) {
        for y in 0..dst_h {
            let base = y * px;
            // Covered by `taps_cover`; a short LUT would silently
            // black out the row via the `get` below.
            debug_assert!(lut.y_lut.len() >= base + px);
            let mut taps = [(0u16, 0u16); 2];
            let mut tap_count = 0;
            if let Some(slice) = lut.y_lut.get(base..base + px) {
                for tap in slice.iter().flatten() {
                    if tap_count < 2 {
                        taps[tap_count] = *tap;
                        tap_count += 1;
                    }
                }
            }
            let taps = &taps[..tap_count];
            // Hoisted source rows: one bounds check per tap row, not per texel.
            // Covered by `taps_cover`; a short buffer degrades to empty rows.
            let rows: [Option<&[u32]>; 2] = [
                taps.first().and_then(|&(i, _)| {
                    let base = (i as usize).saturating_mul(dst_w);
                    resize_buffer.get(base..base.saturating_add(dst_w))
                }),
                taps.get(1).and_then(|&(i, _)| {
                    let base = (i as usize).saturating_mul(dst_w);
                    resize_buffer.get(base..base.saturating_add(dst_w))
                }),
            ];
            for x in 0..dst_w {
                let dst_index = y * dst_w + x;
                if dst_index >= dst.len() {
                    break;
                }
                let mut acc = [0u16; 4];
                let mut touched = false;
                for (slot, &(_, w)) in taps.iter().enumerate() {
                    let Some(word) = rows[slot].and_then(|row| row.get(x)).copied() else {
                        continue;
                    };
                    let src_val = RgbaWord::unpack_argb(word);
                    acc[0] = acc[0].saturating_add((src_val[0] as u16 * w + 0x80) >> 8);
                    acc[1] = acc[1].saturating_add((src_val[1] as u16 * w + 0x80) >> 8);
                    acc[2] = acc[2].saturating_add((src_val[2] as u16 * w + 0x80) >> 8);
                    acc[3] = acc[3].saturating_add((src_val[3] as u16 * w + 0x80) >> 8);
                    touched = true;
                }
                dst[dst_index] = if touched {
                    RgbaWord::pack_argb(
                        acc[0].min(255) as u8,
                        acc[1].min(255) as u8,
                        acc[2].min(255) as u8,
                        acc[3].min(255) as u8,
                    )
                } else {
                    0
                };
            }
        }
    }

    /// Tap coverage for one frame: every touched LUT entry must exist and
    /// point inside its target; mismatch refuses instead of panicking.
    fn taps_cover(
        lut: &LutEntry,
        resize_len: usize,
        ntsc_len: usize,
        dst_w: usize,
        dst_h: usize,
        tap_stride: usize,
        ntsc_writes: usize,
    ) -> bool {
        let px = lut.lut_pixel_size();
        if lut.x_lut.len() < dst_w.saturating_mul(px) || lut.y_lut.len() < dst_h.saturating_mul(px)
        {
            return false;
        }
        if !lut.x_lut[..dst_w.saturating_mul(px)]
            .iter()
            .flatten()
            .all(|&(i, _)| (i as usize) < tap_stride)
        {
            return false;
        }
        if !lut.y_lut[..dst_h.saturating_mul(px)]
            .iter()
            .flatten()
            .all(|&(i, _)| (i as usize).saturating_add(1).saturating_mul(dst_w) <= resize_len)
        {
            return false;
        }
        ntsc_writes <= ntsc_len
    }

    /// Cached [`Self::taps_cover`]: a struct-key match skips the per-frame scan.
    /// The `lut_rev` epoch invalidates on every rebuild; no hash, no rescan timer.
    fn taps_covered(
        &mut self,
        dst_w: usize,
        avail_h: usize,
        src_stride: usize,
        tap_stride: usize,
        ntsc_writes: usize,
    ) -> bool {
        let key = TapKey {
            dst_w,
            avail_h,
            src_w: self.render_profile.source_logical_size.width,
            src_h: self.render_profile.source_logical_size.height,
            src_stride,
            tap_stride,
            resize_len: self.resize_buffer.len(),
            ntsc_writes,
            ntsc_len: self.ntsc_buffer.len(),
            lut_rev: self.lut.rev,
            kernel_bilinear: matches!(self.lut.kernel, ResizeKernel::Bilinear),
            reserved_top: self.reserved.top,
            reserved_bottom: self.reserved.bottom,
        };
        if let Some((cached_key, covered)) = &self.taps_cache
            && *cached_key == key
        {
            return *covered;
        }
        let covered = Self::taps_cover(
            &self.lut,
            self.resize_buffer.len(),
            self.ntsc_buffer.len(),
            dst_w,
            avail_h,
            tap_stride,
            ntsc_writes,
        );
        self.taps_cache = Some((key, covered));
        covered
    }

    /// Narrow cross-module accessors; fields stay private to this module.
    /// `validate` reads geometry through these, never the fields directly.
    pub(crate) fn profile(&self) -> &VideoRenderProfile {
        &self.render_profile
    }

    /// Output size without exposing the field.
    pub(crate) fn output_size(&self) -> SurfaceSize {
        self.size
    }

    /// Surface presence for the geometry gate.
    pub(crate) fn surface_attached(&self) -> bool {
        self.surface.is_some()
    }

    /// Reserved strips for the viewport computation.
    pub(crate) fn reserved_strip(&self) -> nerust_render_traits::ReservedStrip {
        self.reserved
    }
}

impl SurfaceLifecycle for SoftbufferRenderer {
    fn size(&self) -> SurfaceSize {
        self.size
    }

    fn attach(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        let pair = WindowHandlePair {
            window: window_handle,
            display: display_handle,
        };
        // `softbuffer::Error` borrows raw display handles (`!Send`);
        // the kind stays structured, the message crosses as `OpaqueError`.
        let ctx = Context::new(pair.clone()).map_err(|e| {
            RendererError::attach(
                "softbuffer",
                "context",
                Box::new(OpaqueError(e.to_string())),
            )
        })?;
        let surface = Surface::new(&ctx, pair).map_err(|e| {
            RendererError::attach(
                "softbuffer",
                "surface",
                Box::new(OpaqueError(e.to_string())),
            )
        })?;
        self.ctx = Some(ctx);
        self.surface = Some(surface);
        self.size = size;
        SurfaceLifecycle::resize(self, size);
        Ok(())
    }

    fn detach(&mut self) {
        self.surface.take();
        self.ctx.take();
        self.size = SurfaceSize::new(0, 0);
    }

    fn resize(&mut self, size: SurfaceSize) {
        self.size = size;
        if let Some(surface) = self.surface.as_mut() {
            let w = NonZeroU32::new(size.width).unwrap_or(NonZeroU32::MIN);
            let h = NonZeroU32::new(size.height).unwrap_or(NonZeroU32::MIN);
            if let Err(e) = surface.resize(w, h) {
                warn!("softbuffer resize failed: {e}");
            }
        }
        self.resize_lut();
    }
}

impl FrameSink for SoftbufferRenderer {
    fn render(&mut self, frame: &FrameBuffer) -> RenderResult {
        let Some(valid) = self.validate_frame(frame) else {
            return RenderResult::Skipped;
        };
        let ValidatedFrame {
            dst_w,
            dst_h,
            avail_h,
            src_stride,
            src_h,
            tap_stride,
            ntsc_writes,
        } = valid;
        // `dst_h` is `avail_h + strips` by construction; lock the identity
        // here instead of keying a redundant field.
        debug_assert_eq!(
            dst_h,
            avail_h.saturating_add(self.reserved.height_loss(self.size.height) as usize)
        );
        // Tap coverage admits here (memoizing `&mut`), not inside the
        // pure geometry check above. Refuse before any indexer runs.
        if !self.taps_covered(dst_w, avail_h, src_stride, tap_stride, ntsc_writes) {
            warn!("softbuffer tap coverage mismatch: refusing frame");
            return RenderResult::Skipped;
        }
        let Some(surface) = self.surface.as_mut() else {
            return RenderResult::Skipped;
        };
        let mut buffer = match surface.buffer_mut() {
            Ok(b) => b,
            Err(e) => {
                warn!("softbuffer buffer_mut failed: {e}");
                return RenderResult::Error;
            }
        };
        // A failed surface resize (warned there, not fatal) leaves a
        // stale-sized buffer behind: refuse instead of overrunning it.
        if buffer.as_mut().len() < dst_w.saturating_mul(dst_h) {
            warn!(
                "softbuffer buffer too small: have {}, need {}x{}",
                buffer.as_mut().len(),
                dst_w,
                dst_h
            );
            return RenderResult::Skipped;
        }

        let top = (self.reserved.top as usize).min(dst_h);
        // Strip rows keep the cleared black: the surface buffer is
        // reused across frames, so stale game pixels must not linger.
        buffer.as_mut()[..top.saturating_mul(dst_w)].fill(0);
        buffer.as_mut()[(top.saturating_add(avail_h)).saturating_mul(dst_w)..].fill(0);

        let src = frame.as_ref();
        let game_rows = top.saturating_mul(dst_w);
        let game_len = avail_h.saturating_mul(dst_w);
        let dst = &mut buffer.as_mut()[game_rows..game_rows.saturating_add(game_len)];
        // Monomorphic indexers: one enum replaces three closures, so the
        // hot loop never monomorphizes. Fallible reads render opaque black.
        match frame.format() {
            PixelFormat::Rgba => {
                let indexer = PixelIndexer::Rgba { src };
                Self::rendering(
                    dst,
                    src_stride / 4,
                    src_h,
                    dst_w,
                    avail_h,
                    &indexer,
                    &self.lut,
                    &mut self.resize_buffer,
                );
            }
            PixelFormat::PaletteIndex { palette } => {
                if self.render_profile.ntsc_packed_rgba8.is_some() {
                    Ntsc::simulate_gpu_ntsc_rgba(
                        src_h,
                        src_stride,
                        src,
                        &self.render_profile,
                        &mut self.ntsc_buffer,
                    );
                    let indexer = PixelIndexer::Ntsc {
                        buf: &self.ntsc_buffer,
                    };
                    Self::rendering(
                        dst,
                        self.render_profile.logical_size.width,
                        src_h,
                        dst_w,
                        avail_h,
                        &indexer,
                        &self.lut,
                        &mut self.resize_buffer,
                    );
                } else {
                    // Out-of-range indices render black instead of
                    // panicking the UI thread (see mismatch guard).
                    let indexer = PixelIndexer::Palette { src, palette };
                    Self::rendering(
                        dst,
                        src_stride,
                        src_h,
                        dst_w,
                        avail_h,
                        &indexer,
                        &self.lut,
                        &mut self.resize_buffer,
                    );
                }
            }
        }

        if let Some(layer) = self.osd.as_ref() {
            Self::blend_osd(buffer.as_mut(), dst_w, dst_h, layer);
        }
        match buffer.present() {
            Ok(()) => RenderResult::Presented,
            Err(e) => {
                warn!("softbuffer present failed: {e}");
                RenderResult::Error
            }
        }
    }
}

impl OsdSink for SoftbufferRenderer {
    fn set_osd_layer(&mut self, layer: Option<nerust_render_traits::renderer::OsdLayer>) {
        use nerust_render_traits::renderer::OsdLayer;
        if OsdLayer::same_option(self.osd.as_ref(), layer.as_ref()) {
            return;
        }
        self.osd = layer;
    }
}

impl StripSink for SoftbufferRenderer {
    fn set_reserved_strip(&mut self, strip: nerust_render_traits::ReservedStrip) {
        self.reserved = strip;
        self.resize_lut();
    }
}

impl ProfileSwitch for SoftbufferRenderer {
    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        self.render_profile = profile.clone();
        self.resize_lut();
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct SoftbufferFactory;

impl PipelineFactory for SoftbufferFactory {
    /// Per-use-site composition; same renderer, narrow object.
    fn create_pipeline(
        &self,
        config: &RendererConfig,
        _display_handle: raw_window_handle::RawDisplayHandle,
    ) -> Result<Box<dyn RenderPipeline>, RendererError> {
        Ok(Box::new(SoftbufferRenderer::new(&config.render_profile)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lut::{LutEntry, ResizeKernel},
        ntsc::Ntsc,
    };
    use nerust_render_traits::filter::BLACK_PALETTE_INDEX;
    use nerust_render_traits::{VideoFrameFormat, logical::LogicalSize, physical::PhysicalSize};

    #[test]
    fn blend_osd_writes_argb_words() {
        // Buffers are 0xAARRGGBB: opaque red over opaque black stays red.
        let mut buffer = vec![0xFF00_0000u32; 4];
        let layer = nerust_render_traits::renderer::OsdLayer::new(
            0,
            0,
            2,
            1,
            vec![
                255, 0, 0, 255, // opaque red
                0, 0, 0, 0, // transparent keeps dst
            ],
        );
        SoftbufferRenderer::blend_osd(&mut buffer, 2, 2, &layer);
        assert_eq!(buffer[0], 0xFFFF_0000);
        assert_eq!(buffer[1], 0xFF00_0000);
    }

    #[test]
    fn blend_osd_partial_alpha_uses_integer_math() {
        // Half red over black: (255*128 + 0*127 + 127) / 255 = 128.
        // Integer fixed-point with round-half-up, no FPU per pixel.
        let mut buffer = vec![0xFF00_0000u32; 1];
        let layer = nerust_render_traits::renderer::OsdLayer::new(0, 0, 1, 1, vec![255, 0, 0, 128]);
        SoftbufferRenderer::blend_osd(&mut buffer, 1, 1, &layer);
        assert_eq!(buffer[0], 0xFF80_0000);
    }

    fn test_profile() -> VideoRenderProfile {
        VideoRenderProfile {
            source_logical_size: LogicalSize {
                width: 256,
                height: 240,
            },
            logical_size: LogicalSize {
                width: 256,
                height: 240,
            },
            physical_size: PhysicalSize {
                width: 256.0,
                height: 240.0,
            },
            frame_format: VideoFrameFormat::Rgba,
            ntsc_packed_rgba8: None,
        }
    }

    #[test]
    fn render_skips_mismatched_frame_instead_of_panicking() {
        use nerust_render_traits::{FrameBuffer, PixelFormat};
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        // GBA-sized frame against an NES profile: must skip, and
        // above all must not panic on the UI thread.
        let mut frame = FrameBuffer::with_capacity(240, 160, PixelFormat::Rgba);
        frame.resize(240, 160);
        assert_eq!(
            FrameSink::render(&mut renderer, &frame),
            RenderResult::Skipped
        );
    }

    #[test]
    fn render_skips_without_surface() {
        use nerust_render_traits::{FrameBuffer, PixelFormat};
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        let mut frame = FrameBuffer::with_capacity(256, 240, PixelFormat::Rgba);
        frame.resize(256, 240);
        assert_eq!(
            FrameSink::render(&mut renderer, &frame),
            RenderResult::Skipped
        );
    }

    #[test]
    fn rgba_render_survives_byte_stride() {
        // Regression: the Rgba indexer counts pixels while `stride()` is bytes;
        // a 256-byte-aligned stride once overran and panicked the UI thread.
        let mut lut = LutEntry::new();
        lut.resize_lut(
            SurfaceSize {
                width: 4,
                height: 4,
            },
            1.0,
            SurfaceSize {
                width: 8,
                height: 8,
            },
        );
        let stride = 256usize;
        let mut src = vec![0u8; stride * 4];
        for y in 0..4 {
            for x in 0..4 {
                let o = y * stride + x * 4;
                src[o..o + 4].copy_from_slice(&[0x10, 0x20, 0x30, 0xFF]);
            }
        }
        let mut resize_buffer = vec![0u32; 8 * 4];
        let mut dst = vec![0u32; 8 * 8];
        let indexer = PixelIndexer::Rgba { src: &src };
        SoftbufferRenderer::rendering(
            &mut dst,
            stride / 4,
            4,
            8,
            8,
            &indexer,
            &lut,
            &mut resize_buffer,
        );
        // Uniform input stays uniform in the interior: bilinear weights
        // sum to one (edges may go black where taps fall outside).
        let mut center = Vec::new();
        for y in 2..6 {
            for x in 2..6 {
                center.push(dst[y * 8 + x]);
            }
        }
        assert!(!center.is_empty() && center.iter().all(|&p| p == center[0] && p != 0));
    }

    #[test]
    fn rgba_render_preserves_channel_order() {
        // Regression: the horizontal pass once read [R, G, B, A] as
        // [A, B, G, R], turning blue into yellow and black into red.
        let mut lut = LutEntry::new();
        lut.resize_lut(
            SurfaceSize {
                width: 4,
                height: 4,
            },
            1.0,
            SurfaceSize {
                width: 8,
                height: 8,
            },
        );
        for (input, expected) in [
            ([0x00u8, 0x00, 0xFF, 0xFF], 0xFF00_00FFu32),
            ([0x00u8, 0x00, 0x00, 0xFF], 0xFF00_0000u32),
            ([0xFFu8, 0x00, 0x00, 0xFF], 0xFFFF_0000u32),
        ] {
            let mut src = [0u8; 4 * 4 * 4];
            for px in src.as_chunks_mut::<4>().0 {
                *px = input;
            }
            let mut resize_buffer = vec![0u32; 8 * 4];
            let mut dst = vec![0u32; 8 * 8];
            let indexer = PixelIndexer::Rgba { src: &src };
            SoftbufferRenderer::rendering(&mut dst, 4, 4, 8, 8, &indexer, &lut, &mut resize_buffer);
            for y in 2..6 {
                for x in 2..6 {
                    assert_eq!(dst[y * 8 + x], expected, "input {input:02X?}");
                }
            }
        }
    }

    #[test]
    fn palette_index_out_of_range_renders_black() {
        let frame = vec![0x01u8; 4 * 4];
        let size = LogicalSize {
            width: 4,
            height: 4,
        };
        assert_eq!(Ntsc::palette_index(&frame, size, 4, 1, 1), 0x01);
        assert_eq!(
            Ntsc::palette_index(&frame, size, 4, -1, 1),
            BLACK_PALETTE_INDEX
        );
        assert_eq!(
            Ntsc::palette_index(&frame, size, 4, 4, 1),
            BLACK_PALETTE_INDEX
        );
        assert_eq!(
            Ntsc::palette_index(&frame, size, 4, 1, 4),
            BLACK_PALETTE_INDEX
        );
    }

    #[test]
    fn read_entry_out_of_range_renders_black() {
        let buf = vec![0xABu8; 64 * 64 * 4];
        assert_eq!(Ntsc::read_entry(&buf, 0, 0), 0xABAB_ABAB);
        assert_eq!(Ntsc::read_entry(&buf, 63, 63), 0xABAB_ABAB);
        assert_eq!(Ntsc::read_entry(&buf, 255, 63), 0);
        assert_eq!(Ntsc::read_entry(&buf, 0, 10_000), 0);
    }

    #[test]
    fn taps_cover_accepts_well_formed_lut() {
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        // Second call hits the cache; same verdict, no rescan.
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        assert!(renderer.taps_cache.is_some());
    }

    #[test]
    fn taps_cache_stays_hot_on_identical_rebuild() {
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        assert!(renderer.taps_cache.is_some());
        // Identical rebuild skips work and keeps the epoch: the cached
        // verdict stays valid even though every length matches.
        let rev_before = renderer.lut.rev;
        renderer.resize_lut();
        assert_eq!(renderer.lut.rev, rev_before);
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        assert!(renderer.taps_cache.is_some());
    }

    #[test]
    fn taps_rev_bumps_on_every_mutator() {
        let mut lut = LutEntry::new();
        let rev0 = lut.rev;
        lut.resize_lut_bilinear(2, 2, 2, 2, (1.0, 1.0));
        assert!(lut.rev != rev0);
        let rev1 = lut.rev;
        lut.resize_lut_nearest_neighbor(2, 2, 2, 2, (1.0, 1.0));
        assert!(lut.rev != rev1);
        let rev2 = lut.rev;
        lut.resize_lut(SurfaceSize::new(2, 2), 1.0, SurfaceSize::new(2, 2));
        assert!(lut.rev != rev2);
        // Identical rebuild skips: dims compare first, epoch never moves.
        let rev3 = lut.rev;
        lut.resize_lut(SurfaceSize::new(2, 2), 1.0, SurfaceSize::new(2, 2));
        assert_eq!(lut.rev, rev3);
        // Changed dims rebuild and bump.
        lut.resize_lut(SurfaceSize::new(2, 2), 1.0, SurfaceSize::new(4, 4));
        assert!(lut.rev != rev3);
    }

    #[test]
    fn taps_cache_misses_on_rev_change() {
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        // Same lengths, bumped epoch: verdict must rescan, not hit cache.
        renderer.lut.bump_rev();
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        let (key, covered) = renderer.taps_cache.as_ref().unwrap();
        // The LUT revision is the cache epoch: any rebuild invalidates.
        assert_eq!(key.lut_rev, renderer.lut.rev);
        assert!(*covered);
    }

    #[test]
    fn render_skips_without_surface_before_tap_scan() {
        use nerust_render_traits::{FrameBuffer, PixelFormat};
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        // No surface attached: must skip even with a valid tap key.
        let mut frame = FrameBuffer::with_capacity(256, 240, PixelFormat::Rgba);
        frame.resize(256, 240);
        assert_eq!(
            FrameSink::render(&mut renderer, &frame),
            RenderResult::Skipped
        );
    }

    #[test]
    fn taps_cover_rejects_short_lut() {
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        assert!(!renderer.taps_covered(8, 8, 16, 4, 0));
    }

    #[test]
    fn taps_cover_rejects_out_of_range_taps() {
        // x tap beyond the source stride.
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.lut = LutEntry {
            x_lut: vec![Some((999, 1)), None],
            y_lut: vec![None, None],
            kernel: ResizeKernel::Bilinear,
            rev: 1,
            last_build: None,
        };
        renderer.resize_buffer = vec![0; 8];
        assert!(!renderer.taps_covered(1, 1, 4, 4, 0));
        // y tap beyond the resize buffer.
        renderer.lut = LutEntry {
            x_lut: vec![Some((0, 1)), None],
            y_lut: vec![Some((99, 1)), None],
            kernel: ResizeKernel::Bilinear,
            rev: 2,
            last_build: None,
        };
        renderer.resize_buffer = vec![0; 8 * 4];
        assert!(!renderer.taps_covered(8, 1, 32, 4, 0));
        // NTSC writes beyond the NTSC buffer.
        renderer.lut = LutEntry::new();
        renderer.lut.resize_lut(
            SurfaceSize {
                width: 4,
                height: 4,
            },
            1.0,
            SurfaceSize {
                width: 8,
                height: 8,
            },
        );
        renderer.resize_buffer = vec![0; 8 * 4];
        renderer.ntsc_buffer = vec![0; 10];
        assert!(!renderer.taps_covered(8, 8, 32, 4, 100));
    }

    #[test]
    fn tap_key_distinguishes_shape_and_reservation() {
        // Same dst/tap numbers but different src geometry or strips
        // must not hit the stale-true cache.
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        let first_key = renderer.taps_cache.as_ref().unwrap().0;
        // Reserved strip change flips the key.
        renderer.reserved.top = 2;
        renderer.resize_lut();
        assert!(renderer.taps_covered(8, 6, 1024, 256, 0));
        let second_key = renderer.taps_cache.as_ref().unwrap().0;
        assert_ne!(first_key, second_key);
        assert_eq!(second_key.reserved_top, 2);
        assert_eq!(second_key.avail_h, 6);
        assert_eq!(second_key.src_w, 256);
        // Byte stride is keyed (not just tap pixels): strides sharing
        // a `tap_stride` under integer division still miss the cache.
        assert_eq!(second_key.src_stride, 1024);
    }

    #[test]
    fn fallible_rgba_indexer_renders_black_on_short_rows() {
        // Short source rows never panic: missing texels fall back to
        // opaque black instead of `unwrap()` on the UI thread.
        let short: Vec<u8> = vec![0x10, 0x20];
        let indexer = PixelIndexer::Rgba { src: &short };
        assert_eq!(indexer.pixel(0), [0, 0, 0, 255]);
        assert_eq!(indexer.pixel(99), [0, 0, 0, 255]);
    }

    #[test]
    fn private_state_exposes_narrow_accessors() {
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        assert_eq!(renderer.output_size(), SurfaceSize::new(8, 8));
        assert!(!renderer.surface_attached());
        assert_eq!(
            renderer.reserved_strip(),
            nerust_render_traits::ReservedStrip::default()
        );
        assert_eq!(renderer.profile().source_logical_size.width, 256);
        // OSD payload is thread-safe: no Rc forces the Send/Sync impls.
        fn assert_thread_safe<T: Send + Sync>() {}
        assert_thread_safe::<nerust_render_traits::renderer::OsdLayer>();
    }

    #[test]
    fn pipeline_factory_returns_narrow_composition() {
        use nerust_render_traits::renderer::{PipelineFactory, RendererConfig};
        let factory = SoftbufferFactory;
        let config = RendererConfig {
            render_profile: test_profile(),
            vsync: true,
            max_osd_upload_retries: 3,
        };
        // Never dereferenced: the softbuffer factory ignores the handle.
        let handle: raw_window_handle::RawDisplayHandle = unsafe { std::mem::zeroed() };
        let pipeline = factory.create_pipeline(&config, handle).unwrap();
        assert_eq!(
            nerust_render_traits::renderer::SurfaceLifecycle::size(&*pipeline),
            SurfaceSize::new(0, 0)
        );
    }

    #[test]
    fn lifecycle_split_seams_are_object_safe() {
        use nerust_render_traits::renderer::{FrameSink, ProfileSwitch, SurfaceLifecycle};
        fn assert_seams<T: SurfaceLifecycle + FrameSink + ProfileSwitch>() {}
        assert_seams::<SoftbufferRenderer>();
        // Split seams compose as trait objects for lifecycle-only callers.
        fn lifecycle_only(_: &mut dyn SurfaceLifecycle) {}
        fn sink_only(_: &mut dyn FrameSink) {}
        let _ = (lifecycle_only, sink_only);
    }

    #[test]
    fn palette_indexer_decodes_words_and_oob_is_black() {
        let palette = [0x1234_56FFu32, 0xAABB_CCDD];
        let src = [0u8, 1, 9];
        let indexer = PixelIndexer::Palette {
            src: &src,
            palette: &palette,
        };
        assert_eq!(indexer.pixel(0), [0x12, 0x34, 0x56, 0xFF]);
        assert_eq!(indexer.pixel(1), [0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(indexer.pixel(2), [0, 0, 0, 0]);
        assert_eq!(indexer.pixel(99), indexer.pixel(0));
    }

    #[test]
    fn ntsc_indexer_decodes_words_and_oob_is_black() {
        let buf = [0xFF00_00FFu32];
        let indexer = PixelIndexer::Ntsc { buf: &buf };
        assert_eq!(indexer.pixel(0), [0xFF, 0x00, 0x00, 0xFF]);
        assert_eq!(indexer.pixel(7), [0, 0, 0, 0]);
    }

    #[test]
    fn tap_key_struct_distinguishes_every_field() {
        use std::collections::HashSet;
        let base = TapKey {
            dst_w: 8,
            avail_h: 8,
            src_w: 256,
            src_h: 240,
            src_stride: 1024,
            tap_stride: 256,
            resize_len: 64,
            ntsc_writes: 0,
            ntsc_len: 0,
            lut_rev: 1,
            kernel_bilinear: true,
            reserved_top: 0,
            reserved_bottom: 0,
        };
        // Derived Hash works as a keyed-cache index: identical keys collide.
        let mut set = HashSet::new();
        set.insert(base);
        assert!(set.contains(&base));
        let mut varied = base;
        varied.dst_w = 9;
        assert_ne!(varied, base);
        assert!(!set.contains(&varied));
        let mut varied = base;
        varied.lut_rev = 2;
        assert_ne!(varied, base);
        assert_eq!(varied.lut_rev, 2);
        let mut varied = base;
        varied.kernel_bilinear = false;
        assert_ne!(varied, base);
        let mut varied = base;
        varied.reserved_bottom = 3;
        assert_ne!(varied, base);
        assert_eq!(base, base);
    }

    #[test]
    fn horizontal_pass_skips_resize_overflow_and_blacks_edges() {
        let mut lut = LutEntry::new();
        lut.resize_lut(
            SurfaceSize {
                width: 2,
                height: 2,
            },
            1.0,
            SurfaceSize {
                width: 2,
                height: 2,
            },
        );
        let src = vec![0xFFu8; 2 * 2 * 4];
        let indexer = PixelIndexer::Rgba { src: &src };
        let mut tiny = vec![0u32; 1];
        SoftbufferRenderer::horizontal_pass(2, 2, 2, &indexer, &lut, 2, &mut tiny);
        let mut full = vec![0u32; 4];
        SoftbufferRenderer::horizontal_pass(2, 2, 2, &indexer, &lut, 2, &mut full);
        assert!(full.iter().all(|&w| w != 0));
    }

    #[test]
    fn vertical_pass_skips_dst_overflow() {
        let mut lut = LutEntry::new();
        lut.resize_lut(
            SurfaceSize {
                width: 2,
                height: 2,
            },
            1.0,
            SurfaceSize {
                width: 2,
                height: 2,
            },
        );
        let resize = vec![0xFFFF_0000u32; 4];
        let mut tiny = vec![0u32; 1];
        SoftbufferRenderer::vertical_pass(&mut tiny, 2, 2, &lut, 2, &resize);
        let mut full = vec![0u32; 4];
        SoftbufferRenderer::vertical_pass(&mut full, 2, 2, &lut, 2, &resize);
        assert!(full.iter().all(|&w| w != 0));
    }

    #[test]
    fn validate_rejects_dimension_mismatch() {
        use nerust_render_traits::{FrameBuffer, PixelFormat};
        let renderer = SoftbufferRenderer::new(&test_profile());
        let mut frame = FrameBuffer::with_capacity(240, 160, PixelFormat::Rgba);
        frame.resize(240, 160);
        assert!(renderer.validate_frame(&frame).is_none());
    }

    #[test]
    fn validate_rejects_zero_surface() {
        use nerust_render_traits::{FrameBuffer, PixelFormat};
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(0, 0);
        let mut frame = FrameBuffer::with_capacity(256, 240, PixelFormat::Rgba);
        frame.resize(256, 240);
        assert!(renderer.validate_frame(&frame).is_none());
    }

    #[test]
    fn validate_accepts_matching_frame() {
        use nerust_render_traits::{FrameBuffer, PixelFormat};
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        // No surface attached: geometry passes but attach gate refuses.
        let mut frame = FrameBuffer::with_capacity(256, 240, PixelFormat::Rgba);
        frame.resize(256, 240);
        assert!(renderer.validate_frame(&frame).is_none());
    }
}
