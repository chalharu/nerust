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
        GpuFactory, GpuRenderer, OpaqueError, OsdLayer, RenderResult, RendererConfig, RendererError,
    },
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::view::{GlView, OsdOutcome, ViewError};

pub struct GlRenderer {
    display: Display,
    gl_config: glutin::config::Config,
    render_profile: VideoRenderProfile,
    view: Option<GlView>,
    context: Option<glutin::context::PossiblyCurrentContext>,
    gl_surface: Option<glutin::surface::Surface<WindowSurface>>,
    expected_frame_len: usize,
    size: SurfaceSize,
    reserved: nerust_render_traits::ReservedStrip,
    osd: Option<OsdLayer>,
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
        view.use_vao(true);
        view.on_load(profile)
            .map_err(|e| RendererError::new("view", Box::new(e)))?;
        view.set_osd_layer(self.osd.clone());
        Ok(view)
    }

    /// Tear down the live view, warning once on failure. Best-effort:
    /// a failed delete must not fail the rebuild that triggered it.
    fn close_live_view(&mut self) {
        if let Some(ref mut v) = self.view
            && v.on_close()
        {
            log::warn!("GlRenderer: view teardown failed");
        }
    }
}

impl GpuRenderer for GlRenderer {
    fn size(&self) -> SurfaceSize {
        self.size
    }

    fn resize(&mut self, size: SurfaceSize) {
        if let (Some(surf), Some(ctx)) = (self.gl_surface.as_ref(), self.context.as_ref()) {
            let (w, h) = (
                NonZeroU32::new(size.width.max(1)).expect("clamped to at least 1"),
                NonZeroU32::new(size.height.max(1)).expect("clamped to at least 1"),
            );
            surf.resize(ctx, w, h);
        }
        self.size = size;
    }

    /// GL surface is never invalidated on resize — just update viewport size.
    fn reattach(
        &mut self,
        _wh: RawWindowHandle,
        _dh: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.size = size;
        Ok(())
    }

    fn attach(
        &mut self,
        wh: RawWindowHandle,
        _dh: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.size = size;

        // Create GL context and surface.
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
        let (w, h) = (
            NonZeroU32::new(size.width.max(1)).expect("clamped to at least 1"),
            NonZeroU32::new(size.height.max(1)).expect("clamped to at least 1"),
        );
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

        // Proc names are static GL strings without interior nuls.
        GlView::load_with(|n| {
            let c = std::ffi::CString::new(n).expect("GL proc name has no interior nul");
            self.display.get_proc_address(&c)
        });
        self.view = Some(self.fresh_view(&self.render_profile)?);
        self.context = Some(ctx);
        self.gl_surface = Some(surf);
        self.expected_frame_len = frame_byte_len(&self.render_profile);
        Ok(())
    }

    fn detach(&mut self) {
        // Explicit teardown while the context may still be current:
        // bare texture names have no Drop to reclaim them.
        let view = self.view.take();
        match (view, self.context.as_ref()) {
            (Some(mut v), Some(ctx)) if ctx.is_current() => {
                if v.on_close() {
                    log::warn!("GlRenderer: view teardown failed");
                }
                drop(v);
            }
            (Some(v), _) => {
                log::debug!("GlRenderer: detach without current context");
                drop(v);
            }
            (None, _) => {}
        }
        self.context = None;
        drop(self.gl_surface.take());
    }

    fn set_reserved_strip(&mut self, strip: nerust_render_traits::ReservedStrip) {
        self.reserved = strip;
    }

    /// Stage an OSD layer; header gate skips the clone on equality.
    /// Keeps the stored copy needed for profile rebuilds.
    fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
        if osd_layers_equal(self.osd.as_ref(), layer.as_ref()) {
            return;
        }
        if let Some(view) = self.view.as_mut() {
            view.set_osd_layer(layer.clone());
        }
        self.osd = layer;
    }

    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        self.render_profile = profile.clone();
        let Some(ref ctx) = self.context else {
            return Err(RendererError::new(
                "update: not attached",
                Box::new(OpaqueError("".to_string())),
            ));
        };
        if !ctx.is_current() {
            return Err(RendererError::new(
                "update: not current",
                Box::new(OpaqueError("".to_string())),
            ));
        }
        // Build before tearing down: a failed compile keeps the old
        // view live instead of leaving a half-closed one behind.
        let fresh = self.fresh_view(profile)?;
        self.expected_frame_len = frame_byte_len(profile);
        self.close_live_view();
        self.view = Some(fresh);
        Ok(())
    }

    fn render(&mut self, frame_buffer: &FrameBuffer) -> RenderResult {
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

        if let Err(e) = view.on_resize_with_strip(
            self.size.width as i32,
            self.size.height as i32,
            self.reserved,
        ) {
            log::warn!("GlRenderer: resize failed: {e}");
            return RenderResult::Error;
        }
        if let Some(p) = frame_buffer.palette_as_rgba8()
            && let Err(e) = view.update_palette_texture(&p)
        {
            log::warn!("GlRenderer: palette upload failed: {e}");
            return RenderResult::Error;
        }
        // Stride-padded Rgba rows (256-byte aligned) upload as-is
        // via UNPACK_ROW_LENGTH; pre-GLES3 packs into fixed staging
        // like wgpu's pitch-mismatch path. Short data refuses instead
        // of panicking the UI thread.
        let fs = match self.render_profile.frame_format {
            VideoFrameFormat::Rgba => self.render_profile.logical_size,
            VideoFrameFormat::Palette => self.render_profile.source_logical_size,
        };
        let bpp = self.render_profile.frame_format.bytes_per_pixel();
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
        match view.draw_osd(self.size.width as i32, self.size.height as i32) {
            Ok(OsdOutcome::Dropped) => {
                log::warn!("GlRenderer: dropping OSD layer after repeated upload failures");
            }
            Ok(OsdOutcome::Presented) => {}
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

/// Shared OSD equality lives on [`OsdLayer`] (single source).
fn osd_layers_equal(a: Option<&OsdLayer>, b: Option<&OsdLayer>) -> bool {
    OsdLayer::same_option(a, b)
}

/// Expected upload length for a profile: logical pixels for RGBA,
/// source pixels for palette, times bytes per pixel.
fn frame_byte_len(profile: &VideoRenderProfile) -> usize {
    let fs = match profile.frame_format {
        VideoFrameFormat::Rgba => profile.logical_size,
        VideoFrameFormat::Palette => profile.source_logical_size,
    };
    fs.width * fs.height * profile.frame_format.bytes_per_pixel()
}

/// Checked frame geometry shared by the attach guard and the three
/// upload paths: one validated computation, no `usize::MAX` sentinel.
struct FrameLayout {
    width: usize,
    height: usize,
    /// Tight row bytes (`width * bpp`).
    row_bytes: usize,
    /// Source stride in bytes.
    stride: usize,
    /// Source bytes the chosen path consumes.
    need: usize,
    /// Stride in pixels for the `UNPACK_ROW_LENGTH` path.
    stride_px: i32,
}

impl FrameLayout {
    fn checked(
        fs: nerust_render_traits::logical::LogicalSize,
        bpp: usize,
        stride: usize,
        data_len: usize,
        unpack_row_length: bool,
    ) -> Result<Self, ViewError> {
        let row_bytes = fs.width.checked_mul(bpp).ok_or(ViewError::Frame {
            what: "frame dimensions overflow",
            need: 0,
            have: data_len,
        })?;
        let stride_px = stride.checked_div(bpp.max(1)).ok_or(ViewError::Frame {
            what: "frame dimensions overflow",
            need: 0,
            have: data_len,
        })?;
        let need = if unpack_row_length {
            fs.height.checked_sub(1).map_or(Ok(0), |rows| {
                rows.checked_mul(stride)
                    .and_then(|tail| tail.checked_add(row_bytes))
                    .ok_or(ViewError::Frame {
                        what: "frame dimensions overflow",
                        need: 0,
                        have: data_len,
                    })
            })?
        } else if stride == row_bytes {
            fs.width
                .checked_mul(fs.height)
                .and_then(|px| px.checked_mul(bpp))
                .ok_or(ViewError::Frame {
                    what: "frame dimensions overflow",
                    need: 0,
                    have: data_len,
                })?
        } else {
            stride.checked_mul(fs.height).ok_or(ViewError::Frame {
                what: "frame dimensions overflow",
                need: 0,
                have: data_len,
            })?
        };
        if data_len < need {
            return Err(ViewError::Frame {
                what: "frame buffer too small",
                need,
                have: data_len,
            });
        }
        Ok(Self {
            width: fs.width,
            height: fs.height,
            row_bytes,
            stride,
            need,
            stride_px: stride_px as i32,
        })
    }
}

/// Upload one frame through the three stride paths (direct,
/// exact, repacked). Length failures refuse as Frame errors.
fn upload_frame(
    view: &mut GlView,
    tight: &mut Vec<u8>,
    layout: &FrameLayout,
    data: &[u8],
) -> Result<(), ViewError> {
    // Geometry is validated on entry (`FrameLayout::checked`); slicing
    // below cannot fail, so `get(..)` guards stay total by construction.
    if view.supports_unpack_row_length() {
        match data.get(..layout.need) {
            Some(bytes) => view.on_update(bytes, layout.stride_px),
            None => Err(ViewError::Frame {
                what: "frame buffer too small",
                need: layout.need,
                have: data.len(),
            }),
        }
    } else if layout.stride == layout.row_bytes {
        match data.get(..layout.need) {
            Some(bytes) => view.on_update(bytes, layout.width as i32),
            None => Err(ViewError::Frame {
                what: "frame buffer too small",
                need: layout.need,
                have: data.len(),
            }),
        }
    } else {
        if tight.len() != layout.need {
            tight.resize(layout.need, 0);
        }
        pack_rows_stripped(data, layout.stride, layout.height, tight, layout.row_bytes);
        view.on_update(tight, layout.width as i32)
    }
}

// ---------------------------------------------------------------------------
// GlFactory
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct GlFactory;

impl GlFactory {
    fn create_display(dh: RawDisplayHandle) -> Result<Display, glutin::error::Error> {
        use DisplayApiPreference::*;
        #[cfg(all(target_os = "macos", not(target_family = "wasm")))]
        let _preference = Cgl;

        #[cfg(all(
            windows,
            not(target_os = "ios"),
            not(target_os = "macos"),
            not(target_family = "wasm")
        ))]
        let _preference = Wgl(None);

        #[cfg(all(
            unix,
            not(target_os = "ios"),
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_family = "wasm")
        ))]
        let _preference = Glx(Box::new(|_reg| {}));

        #[cfg(all(
            any(windows, unix),
            not(target_os = "ios"),
            not(target_os = "macos"),
            not(target_family = "wasm")
        ))]
        let _preference = Egl;

        #[cfg(all(
            unix,
            not(target_os = "ios"),
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_family = "wasm")
        ))]
        let _preference = EglThenGlx(Box::new(|_reg| {}));

        #[cfg(all(
            windows,
            not(target_os = "ios"),
            not(target_os = "macos"),
            not(target_family = "wasm")
        ))]
        let _preference = EglThenWgl(None);

        unsafe { glutin::display::Display::new(dh, _preference) }
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
                    RendererError::new("no config", Box::new(OpaqueError("".to_string())))
                })?
        };
        Ok(Box::new(GlRenderer {
            display,
            gl_config,
            render_profile: config.render_profile.clone(),
            view: None,
            context: None,
            gl_surface: None,
            expected_frame_len: 0,
            size: SurfaceSize::new(0, 0),
            reserved: nerust_render_traits::ReservedStrip::default(),
            osd: None,
            tight: Vec::new(),
        }))
    }
}

/// Strip source row padding into a tight destination, mirroring
/// wgpu's `pack_frame_rows` pitch-mismatch path. Callers guarantee
/// `source.len() >= source_stride * height`, `destination.len() >=
/// `row_bytes * height`, and nonzero pitches.
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
    use super::{osd_layers_equal, pack_rows_stripped};

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
        use super::FrameLayout;
        use nerust_render_traits::logical::LogicalSize;
        let fs = LogicalSize {
            width: 64,
            height: 64,
        };
        // Tight rows: consumes exactly the frame.
        let tight = FrameLayout::checked(fs, 4, 256, 64 * 64 * 4, false).unwrap();
        assert_eq!((tight.row_bytes, tight.need), (256, 64 * 64 * 4));
        // Padded rows via UNPACK_ROW_LENGTH.
        let padded = FrameLayout::checked(fs, 4, 260, 63 * 260 + 256, true).unwrap();
        assert_eq!(padded.need, 63 * 260 + 256);
        assert_eq!(padded.stride_px, 65);
        // Short data refuses with sizes, overflow without a fake size.
        assert!(FrameLayout::checked(fs, 4, 256, 10, false).is_err());
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
            Err(crate::view::ViewError::Frame { need: 0, .. })
        ));
    }

    #[test]
    fn frame_byte_len_covers_both_pixel_formats() {
        use nerust_render_traits::{
            VideoFrameFormat, logical::LogicalSize, physical::PhysicalSize,
        };
        let profile = |format| nerust_render_traits::VideoRenderProfile {
            frame_format: format,
            source_logical_size: LogicalSize {
                width: 64,
                height: 64,
            },
            logical_size: LogicalSize {
                width: 128,
                height: 128,
            },
            physical_size: PhysicalSize {
                width: 64.0,
                height: 64.0,
            },
            ntsc_packed_rgba8: None,
        };
        assert_eq!(
            super::frame_byte_len(&profile(VideoFrameFormat::Rgba)),
            128 * 128 * 4
        );
        assert_eq!(
            super::frame_byte_len(&profile(VideoFrameFormat::Palette)),
            64 * 64
        );
    }

    #[test]
    fn osd_gate_skips_identical_layers() {
        use nerust_render_traits::renderer::OsdLayer;
        let layer = OsdLayer {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![1; 8],
        };
        assert!(osd_layers_equal(Some(&layer), Some(&layer.clone())));
        assert!(!osd_layers_equal(
            Some(&layer),
            Some(&OsdLayer {
                x: 1,
                ..layer.clone()
            })
        ));
        assert!(osd_layers_equal(None, None));
        assert!(!osd_layers_equal(Some(&layer), None));
    }

    #[test]
    fn frame_error_reports_sizes() {
        let err = super::ViewError::Frame {
            what: "frame buffer too small",
            need: 8,
            have: 2,
        };
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
            let proc_name = std::ffi::CString::new(name).expect("GL proc name has no interior nul");
            display.display.get_proc_address(&proc_name)
        });
        let (display, gl_config) = display.into_parts();
        let profile = gl_test_profile();
        let mut renderer = super::GlRenderer {
            display,
            gl_config,
            render_profile: profile.clone(),
            view: None,
            context: None,
            gl_surface: None,
            expected_frame_len: 0,
            size: SurfaceSize::new(64, 64),
            reserved: nerust_render_traits::ReservedStrip::default(),
            osd: None,
            tight: Vec::new(),
        };
        let empty = nerust_render_traits::FrameBuffer::with_capacity(
            64,
            64,
            nerust_render_traits::PixelFormat::Rgba,
        );
        // Detached states refuse gracefully instead of touching GL.
        assert!(matches!(renderer.render(&empty), RenderResult::Skipped));
        assert!(renderer.update_render_profile(&profile).is_err());
        renderer.set_osd_layer(Some(OsdLayer {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![255; 8],
        }));
        // Equality gate: identical re-stage is a no-op for the stored copy.
        renderer.set_osd_layer(Some(OsdLayer {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![255; 8],
        }));
        // Fresh view build, then an atomic profile rebuild over it.
        let view = renderer.fresh_view(&profile).unwrap();
        renderer.view = Some(view);
        renderer.context = Some(current.context);
        renderer.update_render_profile(&profile).unwrap();
        renderer.resize(SurfaceSize::new(64, 64));
        // Losing currency refuses the rebuild; detach then unwinds.
        let _not_current = renderer.context.take().unwrap().make_not_current();
        assert!(renderer.update_render_profile(&profile).is_err());
        renderer.detach();
        renderer.detach();
    }
}
