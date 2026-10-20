use std::{ffi::CStr, num::NonZeroU32};

use glutin::{
    config::ConfigTemplateBuilder,
    context::{
        ContextApi, ContextAttributesBuilder, NotCurrentGlContext as _,
        PossiblyCurrentGlContext as _, Version,
    },
    display::{Display, DisplayApiPreference, GlDisplay as _},
    surface::{GlSurface as _, SurfaceAttributesBuilder, WindowSurface},
};
use nerust_render_traits::{
    FrameBuffer, SurfaceSize, VideoFrameFormat, VideoRenderProfile,
    renderer::{
        FrameSink, OsdLayer, OsdSink, ProfileSwitch, RenderResult, RendererConfig, RendererError,
        RendererErrorKind, StripSink, SurfaceLifecycle,
    },
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::frame_view::FrameView;
use crate::layout::{FrameError, FrameLayout};
use crate::{error::ViewError, osd_view::OsdOutcome};

/// Null proc name for the never-null fallback: a (never) interior nul maps
/// to a null pointer instead of panicking the attach path.
static EMPTY_PROC_NAME: &CStr = c"";

/// Build a proc loader from a display: interior-nul names fall back to null.
fn proc_loader(display: &Display) -> impl FnMut(&'static str) -> *const std::os::raw::c_void + '_ {
    move |name| {
        std::ffi::CString::new(name)
            .ok()
            .map(|owned| display.get_proc_address(&owned))
            .unwrap_or_else(|| display.get_proc_address(EMPTY_PROC_NAME))
    }
}

pub struct GlRenderer {
    display: Display,
    gl_config: glutin::config::Config,
    render_profile: VideoRenderProfile,
    /// Fresh upload policies for rebuilt overlay views (injectable for tests).
    osd_policy: Box<dyn nerust_render_traits::renderer::OsdUploadPolicyFactory>,
    frame: FrameView,
    osd: crate::osd_view::OsdView,
    surface: crate::surface_view::SurfaceView,
    context: Option<glutin::context::PossiblyCurrentContext>,
    gl_surface: Option<glutin::surface::Surface<WindowSurface>>,
    size: SurfaceSize,
    reserved: nerust_render_traits::ReservedStrip,
    osd_staged: Option<OsdLayer>,
    /// Warn-once for OSD give-up. Reset when a new layer stages so a
    /// later dead driver warns again without spamming per frame.
    osd_drop_warned: bool,
    /// Warn-once for view-teardown failures (see `close_live_view`).
    close_warned: bool,
    /// Fixed-size staging for the pre-GLES3 pack path (see `render`).
    tight: Vec<u8>,
}

impl std::fmt::Debug for GlRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlRenderer")
            .field("attached", &self.gl_surface.is_some())
            .finish_non_exhaustive()
    }
}

impl GlRenderer {
    /// Fresh views with shaders compiled and the staged layer applied.
    /// Builds temporaries first so a failed compile keeps the old views live.
    fn fresh_views(
        &mut self,
        profile: &VideoRenderProfile,
    ) -> Result<(FrameView, crate::osd_view::OsdView), RendererError> {
        let mut frame = FrameView::new();
        frame.set_use_vao(true);
        let is_gles = frame
            .load_frame(profile)
            .map_err(nerust_render_traits::renderer::RendererError::from)?;
        let mut osd = crate::osd_view::OsdView::new();
        osd.set_osd_policy(self.osd_policy.create_policy());
        osd.reset_for_load();
        osd.init_osd(is_gles);
        osd.set_osd_layer(self.osd_staged.clone());
        Ok((frame, osd))
    }

    /// Load path shared by attach and profile rebuilds: tear down while
    /// current, then build the frame and overlay views on the profile.
    fn on_load(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        let _ = self.on_close(nerust_glwrap::CurrentContext::assert_current(true));
        debug_assert_eq!(self.frame.frame_texture, 0);
        self.surface.reset();
        let (frame, osd) = self.fresh_views(profile)?;
        self.frame = frame;
        self.osd = osd;
        Ok(())
    }

    /// Tear down the live views, warning once per failure streak. Best-effort:
    /// a failed delete must not fail the rebuild; later clean teardown re-arms.
    fn close_live_views(&mut self) {
        let current = self
            .context
            .as_ref()
            .map(|ctx| ctx.is_current())
            .unwrap_or(false);
        let report = self.on_close(nerust_glwrap::CurrentContext::assert_current(current));
        self.log_close_report(&report);
    }

    /// Warn-once sink shared by the rebuild and detach teardowns.
    /// A clean teardown re-arms for the next streak.
    fn log_close_report(&mut self, report: &crate::surface_view::CloseReport) {
        if report.is_ok() {
            self.close_warned = false;
            return;
        }
        if self.close_warned {
            return;
        }
        self.close_warned = true;
        log::warn!(
            "GlRenderer: view teardown failed ({} names: textures={}, programs={}, arrays={}, buffers={})",
            report.failed_names(),
            report.failed_textures(),
            report.failed_programs(),
            report.failed_arrays(),
            report.failed_buffers(),
        );
    }

    /// Validated surface size as `i32` for the GL viewport path.
    /// Values above `i32::MAX` skip the frame instead of truncating.
    fn validated_surface_size(&self) -> Option<(i32, i32)> {
        let w = i32::try_from(self.size.width).ok()?;
        let h = i32::try_from(self.size.height).ok()?;
        (w > 0 && h > 0).then_some((w, h))
    }

    /// Non-zero extent for surface creation: saturating, no `expect`.
    fn nonzero_extent(size: SurfaceSize) -> (NonZeroU32, NonZeroU32) {
        let w = NonZeroU32::new(size.width).unwrap_or(NonZeroU32::MIN);
        let h = NonZeroU32::new(size.height).unwrap_or(NonZeroU32::MIN);
        (w, h)
    }

    /// Context attach: create context + window surface + fresh view.
    fn context_attach(
        &mut self,
        wh: RawWindowHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.size = size;
        let nc = unsafe {
            self.display
                .create_context(
                    &self.gl_config,
                    &ContextAttributesBuilder::new()
                        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
                        .build(Some(wh)),
                )
                .map_err(|e| RendererError::attach("gl", "context", Box::new(e)))?
        };
        let (w, h) = Self::nonzero_extent(size);
        let surf = unsafe {
            self.display
                .create_window_surface(
                    &self.gl_config,
                    &SurfaceAttributesBuilder::<WindowSurface>::new().build(wh, w, h),
                )
                .map_err(|e| RendererError::attach("gl", "surface", Box::new(e)))?
        };
        let ctx = nc
            .make_current(&surf)
            .map_err(|e| RendererError::attach("gl", "make_current", Box::new(e)))?;

        FrameView::load_with(proc_loader(&self.display));
        self.on_load(&self.render_profile.clone())?;
        self.context = Some(ctx);
        self.gl_surface = Some(surf);
        Ok(())
    }

    /// Context detach: explicit teardown while current, then drop.
    fn context_detach(&mut self) {
        let current = self
            .context
            .as_ref()
            .map(|ctx| ctx.is_current())
            .unwrap_or(false);
        let current = nerust_glwrap::CurrentContext::assert_current(current);
        if current.is_none() {
            log::debug!("GlRenderer: detach without current context");
        }
        let report = self.on_close(current);
        self.log_close_report(&report);
        self.context = None;
        drop(self.gl_surface.take());
    }

    /// Explicit teardown. `None` (no current context) skips GL calls
    /// and reports each live name as leaked instead of deleting blind.
    fn on_close(
        &mut self,
        current: Option<nerust_glwrap::CurrentContext>,
    ) -> crate::surface_view::CloseReport {
        let mut report = crate::surface_view::CloseReport::default();
        for name in self.frame.close(current) {
            report.leak(name);
        }
        for name in self.osd.close(current) {
            report.leak(name);
        }
        self.surface.reset();
        report
    }

    /// Composite the staged layer over the game image, if any (best-effort).
    /// A GL failure drops the overlay, never the frame.
    fn draw_osd(&mut self, surface_w: i32, surface_h: i32) -> Result<OsdOutcome, ViewError> {
        Self::composite_osd(
            &mut self.osd,
            &mut self.frame,
            &mut self.surface,
            surface_w,
            surface_h,
        )
    }

    /// Overlay composite over three views: viewport memo, composite, memo update.
    fn composite_osd(
        osd: &mut crate::osd_view::OsdView,
        frame: &mut FrameView,
        surface: &mut crate::surface_view::SurfaceView,
        surface_w: i32,
        surface_h: i32,
    ) -> Result<OsdOutcome, ViewError> {
        // Memoized viewport skips the per-frame `GetIntegerv`;
        // empty stages skip all GL work inside the overlay view.
        let viewport = surface.game_viewport();
        let (outcome, applied) = osd.draw_osd(surface_w, surface_h, frame, viewport)?;
        if let Some(viewport) = applied {
            surface.set_game_viewport(viewport);
        }
        Ok(outcome)
    }

    /// Fit the game into the surface minus reserved OSD strips (memoized).
    fn on_resize_with_strip(
        &mut self,
        viewport_width: i32,
        viewport_height: i32,
        strip: nerust_render_traits::ReservedStrip,
    ) -> Result<(), ViewError> {
        Self::resize_views(
            &mut self.frame,
            &mut self.surface,
            viewport_width,
            viewport_height,
            strip,
        )
    }

    /// Memoized resize over two views: identical inputs skip all GL work.
    fn resize_views(
        frame: &mut FrameView,
        surface: &mut crate::surface_view::SurfaceView,
        viewport_width: i32,
        viewport_height: i32,
        strip: nerust_render_traits::ReservedStrip,
    ) -> Result<(), ViewError> {
        // Zero/negative viewports: nothing to draw; skip GL work.
        if viewport_width <= 0 || viewport_height <= 0 {
            return Ok(());
        }
        if surface.resize_hit(viewport_width, viewport_height, strip) {
            return Ok(());
        }
        let game_box = frame.apply_viewport(viewport_width, viewport_height, strip)?;
        surface.record_resize(viewport_width, viewport_height, strip, game_box);
        Ok(())
    }
}

impl GlRenderer {
    /// Per-frame orchestration: viewport, palette, upload, OSD, swap.
    fn orchestrate_render(&mut self, frame_buffer: &FrameBuffer) -> RenderResult {
        let Some((surf_w, surf_h)) = self.validated_surface_size() else {
            log::warn!("GlRenderer: invalid surface size, skipping frame");
            return RenderResult::Skipped;
        };
        let reserved = self.reserved;
        // Copy, don't borrow: the ~10KiB NTSC kernel rode along with
        // every frame before; only `update_render_profile` clones.
        let frame_format = self.render_profile.frame_format;
        let logical_size = self.render_profile.logical_size;
        let source_logical_size = self.render_profile.source_logical_size;
        if self.context.is_none() || self.gl_surface.is_none() {
            return RenderResult::Skipped;
        }
        // Scoped context borrow: the view calls below take `&mut self`.
        {
            let (Some(ctx), Some(surf)) = (self.context.as_ref(), self.gl_surface.as_ref()) else {
                return RenderResult::Skipped;
            };
            if !ctx.is_current()
                && let Err(e) = ctx.make_current(surf)
            {
                log::warn!("GlRenderer: make_current failed: {e}");
                return RenderResult::Error;
            }
        }

        if let Err(e) = self.on_resize_with_strip(surf_w, surf_h, reserved) {
            log::warn!("GlRenderer: resize failed: {e}");
            return RenderResult::Error;
        }
        if let Some(p) = frame_buffer.palette_as_rgba8()
            && let Err(e) = self.frame.update_palette_texture(&p)
        {
            log::warn!("GlRenderer: palette upload failed: {e}");
            return RenderResult::Error;
        }
        let fs = match frame_format {
            VideoFrameFormat::Rgba => logical_size,
            VideoFrameFormat::Palette => source_logical_size,
        };
        let bpp = frame_format.bytes_per_pixel();
        let stride = frame_buffer.stride();
        if fs.width == 0 || fs.height == 0 || stride == 0 {
            log::warn!("GlRenderer: empty frame");
            return RenderResult::Skipped;
        }
        let data = frame_buffer.as_ref();
        let layout = match FrameLayout::checked(
            fs,
            bpp,
            stride,
            data.len(),
            self.frame.supports_unpack_row_length(),
        ) {
            Ok(layout) => layout,
            Err(e) => {
                log::warn!("GlRenderer: frame upload failed: {e}");
                return RenderResult::Error;
            }
        };
        if let Err(e) = upload_frame(&mut self.frame, &mut self.tight, &layout, data) {
            log::warn!("GlRenderer: frame upload failed: {e}");
            return RenderResult::Error;
        }
        match self.draw_osd(surf_w, surf_h) {
            Ok(OsdOutcome::Dropped) => {
                if !self.osd_drop_warned {
                    self.osd_drop_warned = true;
                    log::warn!("GlRenderer: dropping OSD layer after repeated upload failures");
                }
            }
            Ok(OsdOutcome::Presented) | Ok(OsdOutcome::Skipped) => {}
            Err(e) => {
                log::debug!("GlRenderer: OSD composite failed: {e}");
            }
        }
        let (Some(ctx), Some(surf)) = (self.context.as_ref(), self.gl_surface.as_ref()) else {
            return RenderResult::Skipped;
        };
        if let Err(e) = surf.swap_buffers(ctx) {
            log::warn!("GlRenderer: swap_buffers failed: {e}");
            return RenderResult::Error;
        }
        RenderResult::Presented
    }
}

impl SurfaceLifecycle for GlRenderer {
    fn size(&self) -> SurfaceSize {
        self.size
    }

    fn resize(&mut self, size: SurfaceSize) {
        if let (Some(surf), Some(ctx)) = (self.gl_surface.as_ref(), self.context.as_ref()) {
            let (w, h) = Self::nonzero_extent(size);
            surf.resize(ctx, w, h);
        }
        self.size = size;
    }

    fn attach(
        &mut self,
        wh: RawWindowHandle,
        _dh: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.context_attach(wh, size)
    }

    fn detach(&mut self) {
        self.context_detach();
    }
}

impl FrameSink for GlRenderer {
    fn render(&mut self, frame_buffer: &FrameBuffer) -> RenderResult {
        self.orchestrate_render(frame_buffer)
    }
}

impl OsdSink for GlRenderer {
    /// Stage an OSD layer; header gate skips the clone on equality.
    /// Keeps the stored copy needed for profile rebuilds.
    fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
        if OsdLayer::same_option(self.osd_staged.as_ref(), layer.as_ref()) {
            return;
        }
        self.osd.set_osd_layer(layer.clone());
        self.osd_staged = layer;
        // A fresh stage re-arms the give-up warning.
        self.osd_drop_warned = false;
    }
}

impl StripSink for GlRenderer {
    fn set_reserved_strip(&mut self, strip: nerust_render_traits::ReservedStrip) {
        self.reserved = strip;
    }
}

impl ProfileSwitch for GlRenderer {
    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        self.render_profile = profile.clone();
        // The pre-GLES3 pack buffer only ever grows per frame; a
        // profile change is the one safe point to give memory back.
        self.tight.shrink_to_fit();
        let Some(ref ctx) = self.context else {
            return Err(RendererError::not_attached("update"));
        };
        if !ctx.is_current() {
            return Err(RendererError::not_current("update"));
        }
        // Build before tearing down: a failed compile keeps the old
        // views live instead of leaving half-closed ones behind.
        let (frame, osd) = self.fresh_views(profile)?;
        self.close_live_views();
        self.frame = frame;
        self.osd = osd;
        Ok(())
    }
}

/// Upload one frame through the three stride paths (direct, exact, repacked).
/// Length failures refuse as Frame errors; absurd widths error instead of truncating.
fn upload_frame(
    frame: &mut FrameView,
    tight: &mut Vec<u8>,
    layout: &FrameLayout,
    data: &[u8],
) -> Result<(), ViewError> {
    let width_i32 = i32::try_from(layout.width)
        .map_err(|_| ViewError::from(FrameError::StrideTooWide { have: layout.width }))?;
    if frame.supports_unpack_row_length() {
        match data.get(..layout.need) {
            Some(bytes) => frame.on_update(bytes, layout.stride_px),
            None => Err(ViewError::from(FrameError::TooShort {
                need: layout.need,
                have: data.len(),
            })),
        }
    } else if layout.stride == layout.row_bytes {
        match data.get(..layout.need) {
            Some(bytes) => frame.on_update(bytes, width_i32),
            None => Err(ViewError::from(FrameError::TooShort {
                need: layout.need,
                have: data.len(),
            })),
        }
    } else {
        if tight.len() < layout.need {
            tight.reserve_exact(layout.need - tight.len());
            // SAFETY: `pack_rows_stripped` overwrites all `need` bytes below.
            unsafe {
                tight.set_len(layout.need);
            }
        }
        pack_rows_stripped(data, layout.stride, layout.height, tight, layout.row_bytes);
        frame.on_update(tight, width_i32)
    }
}

#[derive(Debug, Default)]
pub struct GlFactory;

impl GlFactory {
    fn create_display(dh: RawDisplayHandle) -> Result<Display, glutin::error::Error> {
        // Single cfg-selected expression: exactly one arm compiles per
        // platform, so no dead Glx/Wgl/Egl shadow survives.
        #[cfg(target_os = "macos")]
        let preference = DisplayApiPreference::Cgl;
        #[cfg(all(target_os = "windows", not(target_family = "wasm")))]
        let preference = DisplayApiPreference::EglThenWgl(None);
        #[cfg(target_os = "android")]
        let preference = DisplayApiPreference::Egl;
        #[cfg(all(
            unix,
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_family = "wasm"),
            not(target_os = "ios")
        ))]
        let preference = DisplayApiPreference::EglThenGlx(Box::new(|_reg| {}));
        #[cfg(not(any(
            target_os = "macos",
            target_os = "windows",
            target_os = "android",
            all(
                unix,
                not(target_os = "macos"),
                not(target_os = "android"),
                not(target_family = "wasm"),
                not(target_os = "ios")
            )
        )))]
        let preference = DisplayApiPreference::Egl;

        unsafe { glutin::display::Display::new(dh, preference) }
    }
}

impl GlFactory {
    /// Shared construction for the narrow factory.
    fn build(
        config: &RendererConfig,
        display_handle: RawDisplayHandle,
    ) -> Result<GlRenderer, RendererError> {
        let display = Self::create_display(display_handle)
            .map_err(|e| RendererError::attach("gl", "display", Box::new(e)))?;
        let template = ConfigTemplateBuilder::new().with_alpha_size(8).build();
        let gl_config = unsafe {
            display
                .find_configs(template)
                .map_err(|e| RendererError::attach("gl", "configs", Box::new(e)))?
                .next()
                .ok_or_else(|| {
                    RendererError::with_kind(
                        "no config",
                        RendererErrorKind::NoConfig,
                        Box::new(RendererErrorKind::NoConfig),
                    )
                })?
        };
        Ok(GlRenderer {
            display,
            gl_config,
            render_profile: config.render_profile.clone(),
            // Injected policy factory: every rebuilt overlay view mints its
            // own budget from the single config knob (impl-owned policy).
            osd_policy: Box::new(crate::upload::RetryPolicyFactory::new(
                config.max_osd_upload_retries,
            )),
            frame: FrameView::new(),
            osd: crate::osd_view::OsdView::new(),
            surface: crate::surface_view::SurfaceView::new(),
            context: None,
            gl_surface: None,
            size: SurfaceSize::new(0, 0),
            reserved: nerust_render_traits::ReservedStrip::default(),
            osd_staged: None,
            osd_drop_warned: false,
            close_warned: false,
            tight: Vec::new(),
        })
    }
}

impl nerust_render_traits::renderer::PipelineFactory for GlFactory {
    /// Per-use-site composition; same renderer, narrow object.
    fn create_pipeline(
        &self,
        config: &RendererConfig,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn nerust_render_traits::renderer::RenderPipeline>, RendererError> {
        Ok(Box::new(Self::build(config, display_handle)?))
    }
}

/// Strip source row padding into a tight destination.
/// Callers guarantee lengths and nonzero pitches.
fn pack_rows_stripped(
    source: &[u8],
    source_stride: usize,
    height: usize,
    destination: &mut [u8],
    row_bytes: usize,
) {
    for (source_row, destination_row) in source
        .chunks_exact(source_stride)
        .zip(destination.chunks_exact_mut(row_bytes))
        .take(height)
    {
        destination_row.copy_from_slice(&source_row[..row_bytes]);
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameView, pack_rows_stripped};
    use crate::{osd_view::OsdView, surface_view::SurfaceView};

    #[test]
    fn pack_rows_stripped_removes_source_padding() {
        let source = [
            1_u8, 2, 3, 4, 99, 99, //
            5, 6, 7, 8, 88, 88,
        ];
        let mut destination = [0_u8; 8];

        pack_rows_stripped(&source, 6, 2, &mut destination, 4);

        assert_eq!(destination, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn frame_layout_validates_all_three_paths() {
        use crate::layout::FrameLayout;
        use nerust_render_traits::logical::LogicalSize;
        let fs = LogicalSize {
            width: 64,
            height: 64,
        };
        let tight = FrameLayout::checked(fs, 4, 256, 64 * 64 * 4, false).unwrap();
        assert_eq!((tight.row_bytes, tight.need), (256, 64 * 64 * 4));
        let padded = FrameLayout::checked(fs, 4, 260, 63 * 260 + 256, true).unwrap();
        assert_eq!(padded.need, 63 * 260 + 256);
        assert_eq!(padded.stride_px, 65);
        assert!(FrameLayout::checked(fs, 4, 256, 10, false).is_err());
        // bpp == 0 refuses instead of masking through `max(1)`.
        assert!(FrameLayout::checked(fs, 0, 256, 64 * 64 * 4, false).is_err());
        // stride not divisible by bpp refuses.
        assert!(FrameLayout::checked(fs, 4, 257, 64 * 64 * 4, false).is_err());
        let overflow = FrameLayout::checked(
            LogicalSize {
                width: usize::MAX,
                height: 2,
            },
            4,
            usize::MAX,
            usize::MAX,
            false,
        );
        assert!(matches!(
            overflow,
            Err(crate::layout::FrameError::DimensionsOverflow { .. })
        ));
    }

    #[test]
    fn lifecycle_split_seams_are_object_safe() {
        use nerust_render_traits::renderer::{
            FrameSink, PipelineFactory, ProfileSwitch, RenderPipeline, SurfaceLifecycle,
        };
        fn assert_seams<T: SurfaceLifecycle + FrameSink + ProfileSwitch>() {}
        assert_seams::<super::GlRenderer>();
        // Per-use-site composition: the same renderer behind narrow traits,
        // minted by the narrow factory entry point.
        fn assert_pipeline<T: RenderPipeline>() {}
        assert_pipeline::<super::GlRenderer>();
        fn assert_factory<T: PipelineFactory>() {}
        assert_factory::<super::GlFactory>();
        fn narrow_only(_: &mut dyn RenderPipeline) {}
        let _ = narrow_only;
    }

    #[test]
    fn surface_size_validation_rejects_i32_overflow() {
        // `validated_surface_size` is `i32::try_from` on both axes:
        // values above `i32::MAX` refuse instead of truncating.
        assert!(i32::try_from(u32::MAX).is_err());
        assert_eq!(i32::try_from(64u32).unwrap(), 64);
    }

    #[test]
    fn nonzero_extent_saturates_without_expect() {
        use nerust_render_traits::SurfaceSize;
        let (w, h) = super::GlRenderer::nonzero_extent(SurfaceSize::new(0, 0));
        assert_eq!((w.get(), h.get()), (1, 1));
        let (w, h) = super::GlRenderer::nonzero_extent(SurfaceSize::new(64, 32));
        assert_eq!((w.get(), h.get()), (64, 32));
    }

    #[test]
    fn osd_gate_skips_identical_layers() {
        use nerust_render_traits::renderer::OsdLayer;
        let layer = OsdLayer::new(0, 0, 2, 1, vec![1; 8]);
        assert!(OsdLayer::same_option(Some(&layer), Some(&layer.clone())));
        assert!(!OsdLayer::same_option(
            Some(&layer),
            Some(&OsdLayer::new(1, 0, 2, 1, layer.rgba.to_vec()))
        ));
        assert!(OsdLayer::same_option(None, None));
        assert!(!OsdLayer::same_option(Some(&layer), None));
    }

    #[test]
    fn frame_error_reports_sizes() {
        let err = super::ViewError::from(crate::layout::FrameError::TooShort { need: 8, have: 2 });
        assert!(err.to_string().contains('8'));
    }

    #[test]
    fn checked_osd_rect_accepts_inside_and_rejects_rest() {
        use crate::osd_overlay::{OSDRect, OsdReject, checked_osd_rect};
        use nerust_render_traits::renderer::OsdLayer;
        let inside = OsdLayer::new(4, 6, 2, 1, vec![1; 8]);
        assert_eq!(
            checked_osd_rect(&inside, 64, 64),
            Ok(OSDRect {
                x: 4,
                y: 6,
                w: 2,
                h: 1,
                stride: 8,
                need: 8,
            })
        );
        let short = OsdLayer::new(inside.x, inside.y, inside.width, inside.height, vec![1; 7]);
        assert_eq!(
            checked_osd_rect(&short, 64, 64),
            Err(OsdReject::Short { need: 8, have: 7 })
        );
        let flat = OsdLayer::new(inside.x, inside.y, inside.width, 0, vec![]);
        assert_eq!(checked_osd_rect(&flat, 64, 64), Err(OsdReject::Empty));
        let outside = OsdLayer::new(
            63,
            inside.y,
            inside.width,
            inside.height,
            inside.rgba.to_vec(),
        );
        assert_eq!(
            checked_osd_rect(&outside, 64, 64),
            Err(OsdReject::OutOfBounds)
        );
        let huge = OsdLayer::new(inside.x, inside.y, u32::MAX, u32::MAX, vec![]);
        assert_eq!(
            checked_osd_rect(&huge, i32::MAX, i32::MAX),
            Err(OsdReject::Empty)
        );
    }

    #[test]
    fn checked_osd_rect_rejects_i32_overflow_as_out_of_bounds() {
        use crate::osd_overlay::{OsdReject, checked_osd_rect};
        use nerust_render_traits::renderer::OsdLayer;
        // u32 coords above i32::MAX refuse instead of wrapping negative.
        let wide = OsdLayer::new(u32::MAX, 0, 2, 1, vec![0; 8]);
        assert_eq!(
            checked_osd_rect(&wide, i32::MAX, i32::MAX),
            Err(OsdReject::OutOfBounds)
        );
    }

    #[test]
    fn osd_gl_y_flips_top_left_to_bottom_left() {
        use crate::osd_overlay::osd_gl_y;
        // 64px surface: top row (y=0,h=1) uploads at GL row 63.
        assert_eq!(osd_gl_y(64, 0, 1), 63);
        assert_eq!(osd_gl_y(64, 6, 1), 57);
        assert_eq!(osd_gl_y(64, 63, 1), 0);
    }

    #[test]
    fn missing_identities_name_program() {
        let err = super::ViewError::MissingUniform {
            name: "unif_matrix",
            program: "game",
        };
        assert!(err.to_string().contains("game"));
        let err = super::ViewError::MissingAttribute {
            name: "position",
            program: "game",
        };
        assert!(err.to_string().contains("position"));
        let err = super::ViewError::OsdUpload {
            rect: (0, 0, 2, 1),
            stage: "blit",
            source: nerust_glwrap::GlError::new(vec![gl::INVALID_VALUE.into()]),
        };
        assert!(err.to_string().contains("blit"));
    }

    #[test]
    fn detects_gles_context_strings() {
        use crate::shader_compiler::is_gles_context;
        assert!(is_gles_context(Some("OpenGL ES 3.2 Mesa 24.1.0")));
        assert!(!is_gles_context(Some("4.6 (Core Profile) Mesa 24.1.0")));
        assert!(!is_gles_context(None));
    }

    #[test]
    fn detects_unpack_row_length_support() {
        use crate::shader_compiler::supports_unpack_row_length;
        assert!(supports_unpack_row_length(Some(
            "4.5 (Core Profile) Mesa 26.0.8"
        )));
        assert!(supports_unpack_row_length(Some(
            "OpenGL ES 3.2 Mesa 24.1.0"
        )));
        assert!(!supports_unpack_row_length(Some(
            "OpenGL ES 2.0 Mesa 24.1.0"
        )));
        assert!(!supports_unpack_row_length(None));
    }

    #[test]
    fn inserts_extra_preamble_after_version_line() {
        use crate::shader_compiler::build_glsl_source;
        let source = build_glsl_source(
            "#version 120\nvoid main(void) {}\n",
            None,
            &["#define TEST 1"],
        );
        assert_eq!(source, "#version 120\n#define TEST 1\nvoid main(void) {}\n");
    }

    #[test]
    fn composes_glsl_parts_in_order() {
        use crate::shader_compiler::compose_glsl_source;
        let source = compose_glsl_source(
            "#version 120",
            &["#define NERUST_MEDIUMP"],
            &["void helper(void) {}\n", "void main(void) {}\n"],
        );

        assert_eq!(
            source,
            "#version 120\n#define NERUST_MEDIUMP\nvoid helper(void) {}\nvoid main(void) {}\n"
        );
    }

    #[test]
    fn ok_gl_passes_values_and_maps_errors() {
        use crate::frame_view::ok_gl;
        assert_eq!(ok_gl::<u32>(Ok(7)).unwrap(), 7);
        let mapped = ok_gl::<u32>(Err(nerust_glwrap::GlError::new(vec![
            gl::INVALID_VALUE.into(),
        ])));
        assert!(matches!(mapped, Err(super::ViewError::Gl(_))));
    }

    #[test]
    fn empty_proc_name_is_static_nul() {
        // Fallback proc name never allocates and never traps on interior nul.
        assert_eq!(super::EMPTY_PROC_NAME.to_bytes(), b"");
    }

    #[test]
    fn resize_views_skips_degenerate_and_memo_hits() {
        use nerust_render_traits::ReservedStrip;
        let mut frame = FrameView::new();
        let mut surface = SurfaceView::new();
        let strip = ReservedStrip::default();
        // Zero viewports skip without touching GL.
        assert!(super::GlRenderer::resize_views(&mut frame, &mut surface, 0, 64, strip).is_ok());
        assert!(super::GlRenderer::resize_views(&mut frame, &mut surface, 64, -1, strip).is_ok());
        // Memo miss without a program fails typed after the pure fit.
        frame.logical_width = 64;
        frame.logical_height = 64;
        assert!(super::GlRenderer::resize_views(&mut frame, &mut surface, 64, 64, strip).is_err());
        // Recorded inputs hit the memo and skip all GL work.
        surface.record_resize(64, 64, strip, [0, 0, 64, 64]);
        assert!(super::GlRenderer::resize_views(&mut frame, &mut surface, 64, 64, strip).is_ok());
    }

    #[test]
    fn composite_osd_skips_empty_stages() {
        let mut frame = FrameView::new();
        let mut osd = OsdView::new();
        let mut surface = SurfaceView::new();
        // Nothing staged: no GL, viewport memo passes through.
        let outcome =
            super::GlRenderer::composite_osd(&mut osd, &mut frame, &mut surface, 64, 64).unwrap();
        assert_eq!(outcome, crate::osd_view::OsdOutcome::Skipped);
    }

    #[test]
    fn upload_frame_refuses_short_and_wide_layouts() {
        use crate::layout::FrameLayout;
        // Short buffer refuses before any GL upload.
        let mut frame = FrameView::new();
        frame.unpack_row_length = true;
        let layout = FrameLayout {
            width: 64,
            height: 64,
            row_bytes: 256,
            stride: 256,
            need: 64 * 64 * 4,
            stride_px: 64,
        };
        assert!(super::upload_frame(&mut frame, &mut Vec::new(), &layout, &[0; 8]).is_err());
        // Absurd width refuses instead of truncating to i32.
        let wide = FrameLayout {
            width: usize::MAX,
            ..layout
        };
        assert!(matches!(
            super::upload_frame(&mut frame, &mut Vec::new(), &wide, &[0; 8]),
            Err(super::ViewError::Frame(
                crate::layout::FrameError::StrideTooWide { .. }
            ))
        ));
        // Tight path packs rows, then fails at the missing program.
        let mut tight_frame = FrameView::new();
        let padded = FrameLayout {
            width: 2,
            height: 1,
            row_bytes: 8,
            stride: 12,
            need: 12,
            stride_px: 3,
        };
        let mut tight = Vec::new();
        assert!(super::upload_frame(&mut tight_frame, &mut tight, &padded, &[7; 12]).is_err());
        // Tight staging holds the packed row; the tail stays zeroed.
        assert_eq!(&tight[..8], &[7; 8]);
    }

    #[test]
    fn pack_rows_stripped_handles_empty_frames() {
        // Zero height iterates nothing; nonzero pitches stay valid.
        let mut destination: [u8; 0] = [];
        pack_rows_stripped(&[], 4, 0, &mut destination, 4);
        assert!(destination.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn osd_policy_factory_mints_independent_budgets() {
        use crate::upload::RetryPolicyFactory;
        use nerust_render_traits::renderer::{
            DEFAULT_OSD_UPLOAD_RETRIES, OsdUploadPolicyFactory as _, RendererConfig,
        };
        let config = RendererConfig {
            render_profile: gl_test_profile(),
            vsync: true,
            max_osd_upload_retries: DEFAULT_OSD_UPLOAD_RETRIES,
        };
        let factory = RetryPolicyFactory::new(config.upload_budget());
        let mut first = factory.create_policy();
        assert!(!first.record_failure());
        assert!(!first.record_failure());
        // A second view starts with a fresh budget, not the spent one.
        let mut second = factory.create_policy();
        assert!(!second.record_failure());
        assert!(!second.record_failure());
        assert!(first.record_failure());
        assert!(second.record_failure());
    }

    #[cfg(unix)]
    fn gl_test_profile() -> nerust_render_traits::VideoRenderProfile {
        use nerust_render_traits::{logical::LogicalSize, physical::PhysicalSize};
        nerust_render_traits::VideoRenderProfile {
            frame_format: nerust_render_traits::VideoFrameFormat::Rgba,
            source_logical_size: LogicalSize {
                width: 64,
                height: 64,
            },
            logical_size: LogicalSize {
                width: 64,
                height: 64,
            },
            physical_size: PhysicalSize {
                width: 64.0,
                height: 64.0,
            },
            ntsc_packed_rgba8: None,
        }
    }

    #[cfg(unix)]
    fn gl_test_layer() -> nerust_render_traits::renderer::OsdLayer {
        nerust_render_traits::renderer::OsdLayer::new(0, 0, 2, 1, vec![255; 8])
    }

    /// Fresh-budget factory for renderer literals in tests (mirrors `GlFactory`).
    #[cfg(unix)]
    fn test_policy_factory() -> Box<dyn nerust_render_traits::renderer::OsdUploadPolicyFactory> {
        use crate::upload::RetryPolicyFactory;
        use nerust_render_traits::renderer::DEFAULT_OSD_UPLOAD_RETRIES;
        Box::new(RetryPolicyFactory::new(DEFAULT_OSD_UPLOAD_RETRIES))
    }

    /// Renderer rebuild on a real context (Xvfb + llvmpipe in CI).
    /// Skips without a display so headless runs stay green.
    #[cfg(unix)]
    #[test]
    fn gl_renderer_rebuild_and_detach() {
        use glutin::context::PossiblyCurrentGlContext as _;
        use nerust_render_traits::renderer::{
            FrameSink as _, OsdSink as _, ProfileSwitch as _, SurfaceLifecycle as _,
        };
        use nerust_render_traits::{
            SurfaceSize,
            renderer::{OsdLayer, RenderResult},
        };

        let _serial = crate::test_display::lock_display();
        let Some(display) = crate::test_display::TestDisplay::connect() else {
            eprintln!("SKIP GL context tests: no GL display");
            return;
        };
        let Some(current) = display.make_pbuffer_current(64, 64) else {
            eprintln!("SKIP GL context tests: no GL context");
            return;
        };
        super::FrameView::load_with(super::proc_loader(&display.display));
        let (display, gl_config) = display.into_parts();
        let profile = gl_test_profile();
        let mut renderer = super::GlRenderer {
            display,
            gl_config,
            render_profile: profile.clone(),
            osd_policy: test_policy_factory(),
            frame: FrameView::new(),
            osd: OsdView::new(),
            surface: SurfaceView::new(),
            context: None,
            gl_surface: None,
            size: SurfaceSize::new(64, 64),
            reserved: nerust_render_traits::ReservedStrip::default(),
            osd_staged: None,
            osd_drop_warned: false,
            close_warned: false,
            tight: Vec::new(),
        };
        let empty = nerust_render_traits::FrameBuffer::with_capacity(
            64,
            64,
            nerust_render_traits::PixelFormat::Rgba,
        );
        assert!(matches!(renderer.render(&empty), RenderResult::Skipped));
        assert!(renderer.update_render_profile(&profile).is_err());
        renderer.set_osd_layer(Some(OsdLayer::new(0, 0, 2, 1, vec![255; 8])));
        renderer.set_osd_layer(Some(OsdLayer::new(0, 0, 2, 1, vec![255; 8])));
        let (frame, osd) = renderer.fresh_views(&profile).unwrap();
        renderer.frame = frame;
        renderer.osd = osd;
        renderer.context = Some(current.context);
        renderer.update_render_profile(&profile).unwrap();
        renderer.resize(SurfaceSize::new(64, 64));
        // Invalid surface skips instead of truncating.
        renderer.resize(SurfaceSize::new(u32::MAX, u32::MAX));
        assert!(matches!(renderer.render(&empty), RenderResult::Skipped));
        renderer.resize(SurfaceSize::new(64, 64));
        let _not_current = renderer.context.take().unwrap().make_not_current();
        assert!(renderer.update_render_profile(&profile).is_err());
        renderer.detach();
        renderer.detach();
    }

    /// Full view drive on a real context (Xvfb + llvmpipe in CI). Skips without a display so
    /// headless runs stay green. Drives the three views directly: no facade stands between them.
    #[cfg(unix)]
    #[test]
    fn gl_views_full_drive() {
        use crate::osd_view::{OsdOutcome, OsdView};
        use nerust_glwrap::CurrentContext;

        let _serial = crate::test_display::lock_display();
        let Some(display) = crate::test_display::TestDisplay::connect() else {
            eprintln!("SKIP GL context tests: no GL display");
            return;
        };
        let Some(_current) = display.make_pbuffer_current(64, 64) else {
            eprintln!("SKIP GL context tests: no GL context");
            return;
        };
        FrameView::load_with(super::proc_loader(&display.display));
        {
            let profile = gl_test_profile();
            let mut frame = FrameView::new();
            let mut osd = OsdView::new();
            let mut surface = SurfaceView::new();
            frame.set_use_vao(true);
            // Load path: close stale, reset memo, build frame, init overlay.
            let load = |frame: &mut FrameView, osd: &mut OsdView, surface: &mut SurfaceView| {
                let _ = frame.close(CurrentContext::assert_current(true));
                surface.reset();
                let is_gles = frame.load_frame(&profile).unwrap();
                osd.reset_for_load();
                osd.init_osd(is_gles);
            };
            load(&mut frame, &mut osd, &mut surface);
            load(&mut frame, &mut osd, &mut surface);
            frame.on_update(&vec![0x80; 64 * 64 * 4], 64).unwrap();
            assert_eq!(
                super::GlRenderer::composite_osd(&mut osd, &mut frame, &mut surface, 64, 64)
                    .unwrap(),
                OsdOutcome::Skipped
            );
            osd.set_osd_layer(Some(gl_test_layer()));
            assert_eq!(
                super::GlRenderer::composite_osd(&mut osd, &mut frame, &mut surface, 64, 64)
                    .unwrap(),
                OsdOutcome::Presented
            );
            assert_eq!(
                super::GlRenderer::composite_osd(&mut osd, &mut frame, &mut surface, 64, 64)
                    .unwrap(),
                OsdOutcome::Presented
            );
            frame.on_update(&vec![0x40; 65 * 64 * 4], 65).unwrap();
            super::GlRenderer::resize_views(
                &mut frame,
                &mut surface,
                64,
                64,
                nerust_render_traits::ReservedStrip::default(),
            )
            .unwrap();
            super::GlRenderer::resize_views(
                &mut frame,
                &mut surface,
                64,
                64,
                nerust_render_traits::ReservedStrip::default(),
            )
            .unwrap();
            let strip = nerust_render_traits::ReservedStrip { top: 8, bottom: 8 };
            super::GlRenderer::resize_views(&mut frame, &mut surface, 64, 64, strip).unwrap();
            let mut before = [0; 4];
            crate::test_display::TestDisplay::viewport_of(&mut before);
            assert_eq!(before[1], 8);
            osd.set_osd_layer(Some(gl_test_layer()));
            assert_eq!(
                super::GlRenderer::composite_osd(&mut osd, &mut frame, &mut surface, 64, 64)
                    .unwrap(),
                OsdOutcome::Presented
            );
            let mut after = [0; 4];
            crate::test_display::TestDisplay::viewport_of(&mut after);
            assert_eq!(before, after);
            frame.update_palette_texture(&[0x12; 256]).unwrap();
            osd.set_osd_layer(None);
            assert_eq!(
                super::GlRenderer::composite_osd(&mut osd, &mut frame, &mut surface, 64, 64)
                    .unwrap(),
                OsdOutcome::Skipped
            );
            // Zero viewport skips without touching GL.
            super::GlRenderer::resize_views(
                &mut frame,
                &mut surface,
                0,
                64,
                nerust_render_traits::ReservedStrip::default(),
            )
            .unwrap();
            let current = CurrentContext::assert_current(true);
            assert!(frame.close(current).is_empty());
            assert!(osd.close(current).is_empty());
            drop(frame);
            drop(osd);
            let ntsc = vec![0xAA; 64 * 42 * 4].into_boxed_slice();
            let pal = gl_test_profile_ntsc(ntsc);
            let mut pframe = FrameView::new();
            pframe.set_use_vao(true);
            let _ = pframe.load_frame(&pal).unwrap();
            pframe.on_update(&vec![7; 64 * 64], 64).unwrap();
            assert!(pframe.close(current).is_empty());
            assert!(pframe.close(current).is_empty());
        }
    }

    /// OSD flip + NTSC byte order, proved by readback (Xvfb + llvmpipe
    /// in CI). Skips without a display so headless runs stay green.
    #[cfg(unix)]
    #[test]
    fn osd_flip_and_ntsc_readback() {
        use crate::frame_textures::{configure_ntsc_texture, upload_texture_bytes, use_texture};
        use crate::osd_view::OsdView;
        use crate::upload::StagingBuf;

        let _serial = crate::test_display::lock_display();
        let Some(display) = crate::test_display::TestDisplay::connect() else {
            eprintln!("SKIP GL context tests: no GL display");
            return;
        };
        let Some(_current) = display.make_pbuffer_current(64, 64) else {
            eprintln!("SKIP GL context tests: no GL context");
            return;
        };
        FrameView::load_with(super::proc_loader(&display.display));
        // 2x2 RGBA texture, zeroed.
        let mut names = [0u32; 2];
        nerust_glwrap::raw::gen_textures(2, names.as_mut_ptr()).unwrap();
        let osd_tex = names[0];
        upload_texture_bytes(0, osd_tex, 2, 2, gl::RGBA as i32, gl::RGBA, &[0; 16]).unwrap();
        // OSD top row (y=0) red, bottom row (y=1) green.
        let red = [255u8, 0, 0, 255, 255, 0, 0, 255];
        let green = [0u8, 255, 0, 255, 0, 255, 0, 255];
        OsdView::blit_bytes(osd_tex, 2, 0, 0, 2, 1, &red).unwrap();
        OsdView::blit_bytes(osd_tex, 2, 0, 1, 2, 1, &green).unwrap();
        let mut out = [0u8; 16];
        nerust_glwrap::raw::get_tex_image(
            gl::TEXTURE_2D,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            out.as_mut_ptr().cast(),
        )
        .unwrap();
        // GL row 0 is bottom-first: the flip put OSD-bottom (green) at row 0;
        // without it both rows would sit swapped.
        assert_eq!(&out[..8], &green);
        assert_eq!(&out[8..], &red);
        // Checked math agrees with the upload sites above.
        assert_eq!(crate::osd_overlay::checked_osd_gl_y(2, 0, 1).unwrap(), 1);
        assert_eq!(crate::osd_overlay::checked_osd_gl_y(2, 1, 1).unwrap(), 0);
        // NTSC kernel: GPU-upload bytes equal the CPU pack order
        // byte-for-byte (big-endian words `[R, G, B, A]`).
        let ntsc_tex = names[1];
        let words: Vec<u8> = (0u8..16).collect();
        let mut scratch = StagingBuf::new();
        configure_ntsc_texture(1, ntsc_tex, 2, 2, &words, &mut scratch).unwrap();
        use_texture(1, ntsc_tex).unwrap();
        let mut ntsc_out = [0u8; 16];
        nerust_glwrap::raw::get_tex_image(
            gl::TEXTURE_2D,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            ntsc_out.as_mut_ptr().cast(),
        )
        .unwrap();
        assert_eq!(&ntsc_out, &words[..]);
        nerust_glwrap::raw::delete_textures(2, names.as_ptr()).unwrap();
    }

    #[cfg(unix)]
    fn gl_test_profile_ntsc(ntsc: Box<[u8]>) -> nerust_render_traits::VideoRenderProfile {
        use nerust_render_traits::{logical::LogicalSize, physical::PhysicalSize};
        nerust_render_traits::VideoRenderProfile {
            frame_format: nerust_render_traits::VideoFrameFormat::Palette,
            source_logical_size: LogicalSize {
                width: 64,
                height: 64,
            },
            logical_size: LogicalSize {
                width: 64,
                height: 64,
            },
            physical_size: PhysicalSize {
                width: 64.0,
                height: 64.0,
            },
            ntsc_packed_rgba8: Some(ntsc),
        }
    }
}
