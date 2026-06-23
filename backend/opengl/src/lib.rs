use std::ffi::CString;
use std::num::NonZeroU32;
use std::os::raw::c_void;

use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
use glutin::display::DisplayApiPreference;
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, WindowSurface};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use nerust_screen_opengl::GlView;
use nerust_screen_video::{
    FrameBuffer, RenderResult, Renderer, SurfaceSize, VideoFrameFormat, VideoRenderProfile,
};

/// OpenGL renderer.
///
/// Two modes:
/// - **Owned** (via [`new()`](Self::new)): creates and manages its own GL
///   context via glutin. Does not need `load_with` / `on_load`.
/// - **Shared** (via [`new_shared()`](Self::new_shared)): works with an
///   externally provided GL context (e.g. GTK GLArea). Call `load_with`,
///   `use_vao`, `on_load` before first use.
pub struct GlRenderer {
    view: GlView,
    expected_frame_len: usize,
    context: Option<glutin::context::PossiblyCurrentContext>,
    surface: Option<glutin::surface::Surface<WindowSurface>>,
}

impl std::fmt::Debug for GlRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlRenderer").finish_non_exhaustive()
    }
}

impl GlRenderer {
    /// Create a new owned renderer that manages its own GL context via glutin.
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new(
        window: &(impl HasWindowHandle + HasDisplayHandle),
        initial_size: SurfaceSize,
        render_profile: &VideoRenderProfile,
    ) -> Result<Self, String> {
        let window_handle = window.window_handle().map_err(|e| e.to_string())?;
        let display_handle = window.display_handle().map_err(|e| e.to_string())?;

        let preference = {
            #[cfg(target_os = "macos")]
            {
                DisplayApiPreference::Glx(Box::new(|_reg| {}))
            }
            #[cfg(target_os = "windows")]
            {
                DisplayApiPreference::Glx(Box::new(|_reg| {}))
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                DisplayApiPreference::EglThenGlx(Box::new(|_reg| {}))
            }
        };

        let display = unsafe {
            glutin::display::Display::new(*display_handle.as_ref(), preference)
                .map_err(|e| format!("failed to create GL display: {e}"))?
        };

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
            .build(Some(*window_handle.as_ref()));

        let not_current = unsafe {
            display
                .create_context(&config, &context_attrs)
                .map_err(|e| format!("failed to create GL context: {e}"))?
        };

        let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            *window_handle.as_ref(),
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
            expected_frame_len,
            context: Some(context),
            surface: Some(surface),
        })
    }

    /// Create a new shared renderer for use with an externally-managed GL
    /// context. Before first use, call `load_with`, `use_vao`, and `on_load`.
    pub fn new_shared() -> Self {
        Self {
            view: GlView::new(),
            expected_frame_len: usize::MAX,
            context: None,
            surface: None,
        }
    }

    /// Load OpenGL function pointers (shared mode).
    pub fn load_with<F: FnMut(&'static str) -> *const c_void>(get_proc_address: F) {
        GlView::load_with(get_proc_address);
    }

    /// Enable or disable vertex array objects (shared mode).
    pub fn use_vao(&mut self, value: bool) {
        self.view.use_vao(value);
    }

    /// Allocate GPU resources for the given render profile (shared mode).
    pub fn on_load(&mut self, render_profile: &VideoRenderProfile) -> Result<(), String> {
        self.view.on_load(render_profile)?;
        let frame_size = match render_profile.frame_format {
            VideoFrameFormat::Rgba => render_profile.logical_size,
            VideoFrameFormat::Palette => render_profile.source_logical_size,
        };
        let bpp = render_profile.frame_format.bytes_per_pixel();
        self.expected_frame_len = frame_size.width * frame_size.height * bpp;
        Ok(())
    }

    /// Release GPU resources (shared mode).
    pub fn on_close(&mut self) {
        self.view.on_close();
    }
}

impl Renderer for GlRenderer {
    fn render(&mut self, frame_buffer: &FrameBuffer) -> RenderResult {
        if let Some(palette_rgba8) = frame_buffer.palette_as_rgba8() {
            self.view.update_palette_texture(&palette_rgba8);
        }
        let bytes = frame_buffer.as_ref();
        let bytes = bytes
            .get(..self.expected_frame_len)
            .expect("GlRenderer expected a loaded frame buffer of the configured size");
        self.view.on_update(bytes.as_ptr());

        if let (Some(context), Some(surface)) = (self.context.as_ref(), self.surface.as_ref()) {
            if let Err(e) = surface.swap_buffers(context) {
                log::warn!("GlRenderer: swap_buffers failed: {e}");
                return RenderResult::Error;
            }
        }
        RenderResult::Presented
    }

    fn reconfigure(&mut self, size: SurfaceSize) {
        self.view.on_resize(size.width as i32, size.height as i32);
    }
}

impl Default for GlRenderer {
    fn default() -> Self {
        Self::new_shared()
    }
}

#[cfg(test)]
mod tests {
    use super::GlRenderer;

    #[test]
    fn default_constructs_without_panic() {
        let _renderer = GlRenderer::default();
    }
}
