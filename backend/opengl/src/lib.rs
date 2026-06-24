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
    FrameBuffer, RenderResult, Renderer, SurfaceSize, VideoFrameFormat, VideoRenderProfile,
};

/// OpenGL renderer with glutin-managed GL context.
pub struct GlRenderer {
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

impl GlRenderer {
    fn create_display(
        display_handle: RawDisplayHandle,
        _raw_window_handle: RawWindowHandle,
    ) -> Result<Display, glutin::error::Error> {
        // Each cfg branch uses only the DisplayApiPreference variant
        // available for that platform's glutin backend feature.
        #[cfg(target_os = "macos")]
        let preference = DisplayApiPreference::Cgl;

        #[cfg(target_os = "windows")]
        let preference = DisplayApiPreference::EglThenWgl(_raw_window_handle);

        // Linux, FreeBSD, Android, iOS, wasm, etc.
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let preference = DisplayApiPreference::Egl;

        unsafe { glutin::display::Display::new(display_handle, preference) }
    }

    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new(
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        initial_size: SurfaceSize,
        render_profile: &VideoRenderProfile,
    ) -> Result<Self, String> {
        let display = Self::create_display(display_handle, window_handle)
            .map_err(|e| format!("failed to create GL display: {e}"))?;

        let template = ConfigTemplateBuilder::new().with_alpha_size(8).build();
        let config = unsafe {
            display
                .find_configs(template)
                .map_err(|e| format!("failed to find GL config: {e}"))?
                .next()
                .ok_or_else(|| "no suitable GL config found".to_string())?
        };

        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .build(Some(window_handle));

        let not_current = unsafe {
            display
                .create_context(&config, &context_attrs)
                .map_err(|e| format!("failed to create GL context: {e}"))?
        };

        let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            window_handle,
            NonZeroU32::new(initial_size.width).unwrap(),
            NonZeroU32::new(initial_size.height).unwrap(),
        );

        let surface = unsafe {
            display
                .create_window_surface(&config, &attrs)
                .map_err(|e| format!("failed to create GL surface: {e}"))?
        };

        let context = not_current
            .make_current(&surface)
            .map_err(|e| format!("failed to make GL context current: {e}"))?;

        GlView::load_with(|name| {
            let cstr = CString::new(name).expect("GL function name contains null byte");
            display.get_proc_address(&cstr)
        });

        let mut view = GlView::new();
        view.use_vao(true);
        view.on_load(render_profile)?;

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
        self.view.on_resize(size.width as i32, size.height as i32);
    }
}
