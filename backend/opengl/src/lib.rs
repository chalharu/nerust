use std::{ffi::CString, num::NonZeroU32};

use glutin::{
    config::ConfigTemplateBuilder,
    context::{
        ContextApi, ContextAttributesBuilder, NotCurrentGlContext as _,
        PossiblyCurrentGlContext as _, Version,
    },
    display::{Display, DisplayApiPreference, GlDisplay as _},
    surface::{GlSurface as _, SurfaceAttributesBuilder, WindowSurface},
};
use nerust_screen_opengl::GlView;
use nerust_screen_video::{
    FrameBuffer, GpuFactory, GpuRenderer, OpaqueError, RenderResult, RendererConfig, RendererError,
    SurfaceSize, VideoFrameFormat, VideoRenderProfile,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// OpenGL renderer: context + shaders + view + surface.
pub struct GlRenderer {
    view: GlView,
    context: glutin::context::PossiblyCurrentContext,
    gl_surface: glutin::surface::Surface<WindowSurface>,
    expected_frame_len: usize,
    size: SurfaceSize,
}

impl std::fmt::Debug for GlRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlRenderer").finish_non_exhaustive()
    }
}

impl Drop for GlRenderer {
    fn drop(&mut self) {
        if !self.context.is_current() {
            let _ = self.context.make_current(&self.gl_surface);
        }
    }
}

impl GpuRenderer for GlRenderer {
    fn size(&self) -> SurfaceSize {
        self.size
    }

    fn attach(
        &mut self,
        _window_handle: RawWindowHandle,
        _display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.size = size;
        Ok(())
    }

    fn detach(&mut self) {}

    fn resize(&mut self, size: SurfaceSize) {
        self.size = size;
    }

    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        if !self.context.is_current()
            && let Err(e) = self.context.make_current(&self.gl_surface)
        {
            return Err(RendererError::new(
                "update: make current",
                Box::new(OpaqueError(e.to_string())),
            ));
        }
        self.view.on_close();
        self.view = GlView::new();
        self.view.use_vao(true);
        self.view
            .on_load(profile)
            .map_err(|e| RendererError::new("view init", Box::new(OpaqueError(e))))?;
        let frame_size = match profile.frame_format {
            VideoFrameFormat::Rgba => profile.logical_size,
            VideoFrameFormat::Palette => profile.source_logical_size,
        };
        let bpp = profile.frame_format.bytes_per_pixel();
        self.expected_frame_len = frame_size.width * frame_size.height * bpp;
        Ok(())
    }

    fn render(&mut self, frame_buffer: &FrameBuffer) -> RenderResult {
        if !self.context.is_current()
            && let Err(e) = self.context.make_current(&self.gl_surface)
        {
            log::warn!("GlRenderer: failed to make context current: {e}");
            return RenderResult::Error;
        }

        self.view
            .on_resize(self.size.width as i32, self.size.height as i32);

        if let Some(palette_rgba8) = frame_buffer.palette_as_rgba8() {
            self.view.update_palette_texture(&palette_rgba8);
        }
        let bytes = frame_buffer.as_ref();
        let bytes = bytes
            .get(..self.expected_frame_len)
            .expect("GlRenderer: frame buffer too small");
        self.view.on_update(bytes.as_ptr());

        if let Err(e) = self.gl_surface.swap_buffers(&self.context) {
            log::warn!("GlRenderer: swap_buffers failed: {e}");
            return RenderResult::Error;
        }
        RenderResult::Presented
    }
}

// ---------------------------------------------------------------------------
// Constructor (called by GlFactory::create_renderer)
// ---------------------------------------------------------------------------

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
                RendererError::new("zero width", Box::new(OpaqueError("...".to_string())))
            })?,
            NonZeroU32::new(initial_size.height).ok_or_else(|| {
                RendererError::new("zero height", Box::new(OpaqueError("...".to_string())))
            })?,
        );

        let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(window_handle, w, h);
        let gl_surface = unsafe {
            display
                .create_window_surface(&config, &attrs)
                .map_err(|e| RendererError::new("create window surface", Box::new(e)))?
        };

        let context = not_current
            .make_current(&gl_surface)
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
            gl_surface,
            expected_frame_len,
            size: initial_size,
        })
    }
}

// ---------------------------------------------------------------------------
// GlFactory
// ---------------------------------------------------------------------------

pub struct GlFactory;

impl Default for GlFactory {
    fn default() -> Self {
        Self
    }
}

impl GpuFactory for GlFactory {
    fn create_renderer(
        &self,
        config: &RendererConfig,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError> {
        GlRenderer::new(
            window_handle,
            display_handle,
            config.initial_size,
            &config.render_profile,
        )
        .map(|r| Box::new(r) as Box<dyn GpuRenderer>)
    }
}
