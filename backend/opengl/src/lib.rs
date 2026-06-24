use std::ffi::CString;
use std::num::NonZeroU32;

use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
use glutin::display::{Display, DisplayApiPreference};
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, WindowSurface};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use nerust_screen_opengl::GlView;
use nerust_screen_video::{
    FrameBuffer, OpaqueError, RenderResult, Renderer, RendererConfig, RendererError,
    RendererFactory, SurfaceSize, VideoFrameFormat, VideoRenderProfile,
};

/// OpenGL renderer with glutin-managed GL context.
pub struct GlRenderer {
    // Drop order: view (glDelete*) → context → surface.
    // `impl Drop` makes the GL context current before the fields drop.
    view: GlView,
    context: glutin::context::PossiblyCurrentContext,
    surface: glutin::surface::Surface<WindowSurface>,
    expected_frame_len: usize,
}

impl std::fmt::Debug for GlRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlRenderer").finish_non_exhaustive()
    }
}

impl Drop for GlRenderer {
    fn drop(&mut self) {
        // Make the GL context current before dropping `view`, so that
        // `GlView`'s `glDelete*` calls run in a valid context.
        if !self.context.is_current() {
            let _ = self.context.make_current(&self.surface);
        }
    }
}

impl GlRenderer {
    fn create_display(
        display_handle: RawDisplayHandle,
        _raw_window_handle: RawWindowHandle,
    ) -> Result<Display, glutin::error::Error> {
        #[cfg(all(target_os = "macos", not(target_family = "wasm")))]
        let _preference = DisplayApiPreference::Cgl;

        #[cfg(all(
            windows,
            not(target_os = "ios"),
            not(target_os = "macos"),
            not(target_family = "wasm")
        ))]
        let _preference = DisplayApiPreference::Wgl(Some(_raw_window_handle));

        #[cfg(all(
            unix,
            not(target_os = "ios"),
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_family = "wasm")
        ))]
        let _preference = DisplayApiPreference::Glx(Box::new(|_reg| {}));

        #[cfg(all(
            any(windows, unix),
            not(target_os = "ios"),
            not(target_os = "macos"),
            not(target_family = "wasm")
        ))]
        let _preference = DisplayApiPreference::Egl;

        #[cfg(all(
            unix,
            not(target_os = "ios"),
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_family = "wasm")
        ))]
        let _preference = DisplayApiPreference::EglThenGlx(Box::new(|_reg| {}));

        #[cfg(all(
            windows,
            not(target_os = "ios"),
            not(target_os = "macos"),
            not(target_family = "wasm")
        ))]
        let _preference = DisplayApiPreference::EglThenWgl(Some(_raw_window_handle));

        unsafe { glutin::display::Display::new(display_handle, _preference) }
    }

    #[allow(clippy::arc_with_non_send_sync)]
    fn new(
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        initial_size: SurfaceSize,
        render_profile: &VideoRenderProfile,
    ) -> Result<Self, RendererError> {
        let display = Self::create_display(display_handle, window_handle)
            .map_err(|e| RendererError::new("display init", Box::new(e)))?;

        let template = ConfigTemplateBuilder::new().with_alpha_size(8).build();
        let config = unsafe {
            display
                .find_configs(template)
                .map_err(|e| RendererError::new("find configs", Box::new(e)))?
                .next()
                .ok_or_else(|| {
                    RendererError::new(
                        "no suitable GL config",
                        Box::new(OpaqueError("no config returned by glutin".to_string())),
                    )
                })?
        };

        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .build(Some(window_handle));

        let not_current = unsafe {
            display
                .create_context(&config, &context_attrs)
                .map_err(|e| RendererError::new("create context", Box::new(e)))?
        };

        let (w, h) = (
            NonZeroU32::new(initial_size.width).ok_or_else(|| {
                RendererError::new(
                    "zero width",
                    Box::new(OpaqueError("surface width must be non-zero".to_string())),
                )
            })?,
            NonZeroU32::new(initial_size.height).ok_or_else(|| {
                RendererError::new(
                    "zero height",
                    Box::new(OpaqueError("surface height must be non-zero".to_string())),
                )
            })?,
        );
        let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(window_handle, w, h);

        let surface = unsafe {
            display
                .create_window_surface(&config, &attrs)
                .map_err(|e| RendererError::new("create window surface", Box::new(e)))?
        };

        let context = not_current
            .make_current(&surface)
            .map_err(|e| RendererError::new("make current", Box::new(e)))?;

        GlView::load_with(|name| {
            let cstr = CString::new(name).expect("GL function name contains null byte");
            display.get_proc_address(&cstr)
        });

        let mut view = GlView::new();
        view.use_vao(true);
        view.on_load(render_profile)
            .map_err(|e| RendererError::new("view init", Box::new(OpaqueError(e))))?;

        let frame_size = match render_profile.frame_format {
            VideoFrameFormat::Rgba => render_profile.logical_size,
            VideoFrameFormat::Palette => render_profile.source_logical_size,
        };
        let bpp = render_profile.frame_format.bytes_per_pixel();
        let expected_frame_len = frame_size.width * frame_size.height * bpp;

        Ok(Self {
            view,
            context,
            surface,
            expected_frame_len,
        })
    }
}

impl Renderer for GlRenderer {
    fn render(&mut self, frame_buffer: &FrameBuffer) -> RenderResult {
        if !self.context.is_current()
            && let Err(e) = self.context.make_current(&self.surface)
        {
            log::warn!("GlRenderer: failed to make GL context current: {e}");
            return RenderResult::Error;
        }

        if let Some(palette_rgba8) = frame_buffer.palette_as_rgba8() {
            self.view.update_palette_texture(&palette_rgba8);
        }
        let bytes = frame_buffer.as_ref();
        let bytes = bytes
            .get(..self.expected_frame_len)
            .expect("GlRenderer expected a loaded frame buffer of the configured size");
        self.view.on_update(bytes.as_ptr());

        if let Err(e) = self.surface.swap_buffers(&self.context) {
            log::warn!("GlRenderer: swap_buffers failed: {e}");
            return RenderResult::Error;
        }
        RenderResult::Presented
    }

    fn reconfigure(&mut self, size: SurfaceSize) {
        if !self.context.is_current()
            && let Err(e) = self.context.make_current(&self.surface)
        {
            log::warn!("GlRenderer: failed to make GL context current in reconfigure: {e}");
            return;
        }
        self.view.on_resize(size.width as i32, size.height as i32);
    }
}

/// `GlRenderer` を構築する Factory。
pub struct GlRendererFactory;

impl RendererFactory for GlRendererFactory {
    fn create_renderer(
        &self,
        config: &RendererConfig,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn Renderer>, RendererError> {
        GlRenderer::new(
            window_handle,
            display_handle,
            config.initial_size,
            &config.render_profile,
        )
        .map(|r| Box::new(r) as Box<dyn Renderer>)
    }
}
