use std::num::NonZeroU32;

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
        GpuFactory, GpuRenderer, OsdLayer, OsdSink, Present, RenderResult, RendererConfig,
        RendererError, RendererErrorKind, StripSink,
    },
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::layout::{FrameError, FrameLayout};
use crate::view::{GlView, OsdOutcome, ViewError};

pub struct GlRenderer {
    display: Display,
    gl_config: glutin::config::Config,
    render_profile: VideoRenderProfile,
    max_osd_upload_retries: u8,
    view: Option<GlView>,
    context: Option<glutin::context::PossiblyCurrentContext>,
    gl_surface: Option<glutin::surface::Surface<WindowSurface>>,
    size: SurfaceSize,
    reserved: nerust_render_traits::ReservedStrip,
    osd: Option<OsdLayer>,
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
    /// Fresh view with shaders compiled and the staged layer applied.
    /// Single construction site for attach and profile rebuilds.
    fn fresh_view(&self, profile: &VideoRenderProfile) -> Result<GlView, RendererError> {
        let mut view = GlView::new();
        view.set_osd_retry_max(self.max_osd_upload_retries);
        view.use_vao(true);
        view.on_load(profile)
            .map_err(|e| RendererError::new("view", Box::new(e)))?;
        view.set_osd_layer(self.osd.clone());
        Ok(view)
    }

    /// Tear down the live view, warning once per failure streak. Best-effort:
    /// a failed delete must not fail the rebuild; later clean teardown re-arms.
    fn close_live_view(&mut self) {
        if let Some(ref mut v) = self.view {
            let current = self
                .context
                .as_ref()
                .map(|ctx| ctx.is_current())
                .unwrap_or(false);
            let report = v.on_close(nerust_glwrap::CurrentContext::assert_current(current));
            self.log_close_report(&report);
        }
    }

    /// Warn-once sink shared by the rebuild and detach teardowns.
    /// A clean teardown re-arms for the next streak.
    fn log_close_report(&mut self, report: &crate::view::CloseReport) {
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
                .map_err(|e| RendererError::new("context", Box::new(e)))?
        };
        let (w, h) = Self::nonzero_extent(size);
        let surf = unsafe {
            self.display
                .create_window_surface(
                    &self.gl_config,
                    &SurfaceAttributesBuilder::<WindowSurface>::new().build(wh, w, h),
                )
                .map_err(|e| RendererError::new("surface", Box::new(e)))?
        };
        let ctx = nc
            .make_current(&surf)
            .map_err(|e| RendererError::new("make_current", Box::new(e)))?;

        // Proc names are static GL strings; a (never) interior nul maps
        // to a null pointer instead of panicking the attach path.
        GlView::load_with(|n| {
            std::ffi::CString::new(n)
                .ok()
                .map(|c| self.display.get_proc_address(&c))
                .unwrap_or(std::ptr::null())
        });
        self.view = Some(self.fresh_view(&self.render_profile)?);
        self.context = Some(ctx);
        self.gl_surface = Some(surf);
        Ok(())
    }

    /// Context detach: explicit teardown while current, then drop.
    fn context_detach(&mut self) {
        let view = self.view.take();
        match (view, self.context.as_ref()) {
            (Some(mut v), Some(ctx)) => {
                let current = nerust_glwrap::CurrentContext::assert_current(ctx.is_current());
                if current.is_none() {
                    log::debug!("GlRenderer: detach without current context");
                }
                let report = v.on_close(current);
                self.log_close_report(&report);
                drop(v);
            }
            (Some(v), None) => {
                log::debug!("GlRenderer: detach without current context");
                drop(v);
            }
            (None, _) => {}
        }
        self.context = None;
        drop(self.gl_surface.take());
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
        // Borrow, don't clone: the ~10KiB NTSC kernel rode along with
        // every frame before; only `update_render_profile` clones.
        let render_profile = &self.render_profile;
        let Some(ref ctx) = self.context else {
            return RenderResult::Skipped;
        };
        let Some(ref surf) = self.gl_surface else {
            return RenderResult::Skipped;
        };
        let Some(ref mut view) = self.view else {
            return RenderResult::Skipped;
        };

        if !ctx.is_current()
            && let Err(e) = ctx.make_current(surf)
        {
            log::warn!("GlRenderer: make_current failed: {e}");
            return RenderResult::Error;
        }

        if let Err(e) = view.on_resize_with_strip(surf_w, surf_h, reserved) {
            log::warn!("GlRenderer: resize failed: {e}");
            return RenderResult::Error;
        }
        if let Some(p) = frame_buffer.palette_as_rgba8()
            && let Err(e) = view.update_palette_texture(&p)
        {
            log::warn!("GlRenderer: palette upload failed: {e}");
            return RenderResult::Error;
        }
        let fs = match render_profile.frame_format {
            VideoFrameFormat::Rgba => render_profile.logical_size,
            VideoFrameFormat::Palette => render_profile.source_logical_size,
        };
        let bpp = render_profile.frame_format.bytes_per_pixel();
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
            view.supports_unpack_row_length(),
        ) {
            Ok(layout) => layout,
            Err(e) => {
                log::warn!("GlRenderer: frame upload failed: {e}");
                return RenderResult::Error;
            }
        };
        if let Err(e) = upload_frame(view, &mut self.tight, &layout, data) {
            log::warn!("GlRenderer: frame upload failed: {e}");
            return RenderResult::Error;
        }
        match view.draw_osd(surf_w, surf_h) {
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
        if let Err(e) = surf.swap_buffers(ctx) {
            log::warn!("GlRenderer: swap_buffers failed: {e}");
            return RenderResult::Error;
        }
        RenderResult::Presented
    }
}

impl Present for GlRenderer {
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

    fn render(&mut self, frame_buffer: &FrameBuffer) -> RenderResult {
        self.orchestrate_render(frame_buffer)
    }
}

impl OsdSink for GlRenderer {
    /// Stage an OSD layer; header gate skips the clone on equality.
    /// Keeps the stored copy needed for profile rebuilds.
    fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
        if OsdLayer::same_option(self.osd.as_ref(), layer.as_ref()) {
            return;
        }
        if let Some(view) = self.view.as_mut() {
            view.set_osd_layer(layer.clone());
        }
        self.osd = layer;
        // A fresh stage re-arms the give-up warning.
        self.osd_drop_warned = false;
    }
}

impl StripSink for GlRenderer {
    fn set_reserved_strip(&mut self, strip: nerust_render_traits::ReservedStrip) {
        self.reserved = strip;
    }
}

impl GpuRenderer for GlRenderer {
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
        // view live instead of leaving a half-closed one behind.
        let fresh = self.fresh_view(profile)?;
        self.close_live_view();
        self.view = Some(fresh);
        Ok(())
    }
}

/// Upload one frame through the three stride paths (direct, exact, repacked).
/// Length failures refuse as Frame errors; absurd widths error instead of truncating.
fn upload_frame(
    view: &mut GlView,
    tight: &mut Vec<u8>,
    layout: &FrameLayout,
    data: &[u8],
) -> Result<(), ViewError> {
    let width_i32 = i32::try_from(layout.width)
        .map_err(|_| ViewError::from(FrameError::StrideTooWide { have: layout.width }))?;
    if view.supports_unpack_row_length() {
        match data.get(..layout.need) {
            Some(bytes) => view.on_update(bytes, layout.stride_px),
            None => Err(ViewError::from(FrameError::TooShort {
                need: layout.need,
                have: data.len(),
            })),
        }
    } else if layout.stride == layout.row_bytes {
        match data.get(..layout.need) {
            Some(bytes) => view.on_update(bytes, width_i32),
            None => Err(ViewError::from(FrameError::TooShort {
                need: layout.need,
                have: data.len(),
            })),
        }
    } else {
        if tight.len() < layout.need {
            tight.resize(layout.need, 0);
        }
        pack_rows_stripped(data, layout.stride, layout.height, tight, layout.row_bytes);
        view.on_update(tight, width_i32)
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

impl GpuFactory for GlFactory {
    fn create_renderer(
        &self,
        config: &RendererConfig,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError> {
        let display = Self::create_display(display_handle)
            .map_err(|e| RendererError::new("display", Box::new(e)))?;
        let template = ConfigTemplateBuilder::new().with_alpha_size(8).build();
        let gl_config = unsafe {
            display
                .find_configs(template)
                .map_err(|e| RendererError::new("configs", Box::new(e)))?
                .next()
                .ok_or_else(|| {
                    RendererError::new("no config", Box::new(RendererErrorKind::NoConfig))
                })?
        };
        Ok(Box::new(GlRenderer {
            display,
            gl_config,
            render_profile: config.render_profile.clone(),
            // Floor of one attempt lives in `RetryBudget::new`, not here.
            max_osd_upload_retries: config.max_osd_upload_retries,
            view: None,
            context: None,
            gl_surface: None,
            size: SurfaceSize::new(0, 0),
            reserved: nerust_render_traits::ReservedStrip::default(),
            osd: None,
            osd_drop_warned: false,
            close_warned: false,
            tight: Vec::new(),
        }))
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
    use super::pack_rows_stripped;

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

    /// Renderer rebuild on a real context (Xvfb + llvmpipe in CI).
    /// Skips without a display so headless runs stay green.
    #[cfg(unix)]
    #[test]
    fn gl_renderer_rebuild_and_detach() {
        use glutin::context::PossiblyCurrentGlContext as _;
        use glutin::display::GlDisplay as _;
        use nerust_render_traits::renderer::{OsdSink as _, Present as _};
        use nerust_render_traits::{
            SurfaceSize,
            renderer::{GpuRenderer as _, OsdLayer, RenderResult},
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
        super::GlView::load_with(|name| {
            std::ffi::CString::new(name)
                .ok()
                .map(|proc_name| display.display.get_proc_address(&proc_name))
                .unwrap_or(std::ptr::null())
        });
        let (display, gl_config) = display.into_parts();
        let profile = gl_test_profile();
        let mut renderer = super::GlRenderer {
            display,
            gl_config,
            render_profile: profile.clone(),
            max_osd_upload_retries: nerust_render_traits::renderer::DEFAULT_OSD_UPLOAD_RETRIES,
            view: None,
            context: None,
            gl_surface: None,
            size: SurfaceSize::new(64, 64),
            reserved: nerust_render_traits::ReservedStrip::default(),
            osd: None,
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
        let view = renderer.fresh_view(&profile).unwrap();
        renderer.view = Some(view);
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
}
