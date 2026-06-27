use std::num::NonZeroU32;

use glutin::{
    config::ConfigTemplateBuilder,
    context::{
        ContextApi, ContextAttributesBuilder, NotCurrentGlContext as _, Version,
    },
    display::{Display, DisplayApiPreference, GlDisplay as _},
    prelude::*,
    surface::{GlSurface as _, SurfaceAttributesBuilder, WindowSurface},
};
use nerust_screen_opengl::GlView;
use nerust_screen_video::{
    FrameBuffer, GpuFactory, GpuRenderer, OpaqueError, RenderResult, RendererConfig,
    RendererError, SurfaceSize, VideoFrameFormat, VideoRenderProfile,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// OpenGL renderer.
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

impl GpuRenderer for GlRenderer {
    fn size(&self) -> SurfaceSize { self.size }
    fn resize(&mut self, size: SurfaceSize) { self.size = size; }

    fn attach(&mut self, _wh: RawWindowHandle, _dh: RawDisplayHandle, size: SurfaceSize) -> Result<(), RendererError> {
        self.size = size;
        Ok(())
    }
    fn detach(&mut self) {}

    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        // View 再初期化 (context は current にしたまま)
        self.view.on_close();
        self.view = GlView::new();
        self.view.use_vao(true);
        self.view.on_load(profile).map_err(|e| RendererError::new("view init", Box::new(OpaqueError(e))))?;
        let fs = match profile.frame_format {
            VideoFrameFormat::Rgba => profile.logical_size,
            VideoFrameFormat::Palette => profile.source_logical_size,
        };
        self.expected_frame_len = fs.width * fs.height * profile.frame_format.bytes_per_pixel();
        Ok(())
    }

    fn render(&mut self, frame_buffer: &FrameBuffer) -> RenderResult {
        if !self.context.is_current()
            && let Err(e) = self.context.make_current(&self.gl_surface)
        {
            log::warn!("GlRenderer: make_current failed: {e}");
            return RenderResult::Error;
        }
        self.view.on_resize(self.size.width as i32, self.size.height as i32);
        if let Some(p) = frame_buffer.palette_as_rgba8() { self.view.update_palette_texture(&p); }
        let bytes = frame_buffer.as_ref();
        let bytes = bytes.get(..self.expected_frame_len).expect("frame buffer too small");
        self.view.on_update(bytes.as_ptr());
        if let Err(e) = self.gl_surface.swap_buffers(&self.context) {
            log::warn!("GlRenderer: swap_buffers failed: {e}");
            return RenderResult::Error;
        }
        RenderResult::Presented
    }
}

impl GlRenderer {}

pub struct GlFactory;
impl Default for GlFactory { fn default() -> Self { Self } }

impl GlFactory {
    fn create_display_unsafe(dh: RawDisplayHandle, wh: RawWindowHandle) -> Result<Display, glutin::error::Error> {
        use DisplayApiPreference::*;
        #[cfg(target_os = "macos")]
        let pref = Cgl;
        #[cfg(all(windows, not(target_os = "macos")))]
        let pref = EglThenWgl(Some(wh));
        #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
        let pref = EglThenGlx(Box::new(|_| {}));
        #[cfg(target_os = "android")]
        let pref = Egl;
        unsafe { glutin::display::Display::new(dh, pref) }
    }
}

impl GpuFactory for GlFactory {
    fn create_renderer(
        &self, config: &RendererConfig, wh: RawWindowHandle, dh: RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError> {
        let display = unsafe { Self::create_display_unsafe(dh, wh) }
            .map_err(|e| RendererError::new("display", Box::new(e)))?;
        let template = ConfigTemplateBuilder::new().with_alpha_size(8).build();
        let gl_config = unsafe {
            display.find_configs(template).map_err(|e| RendererError::new("configs", Box::new(e)))?
                .next().ok_or_else(|| RendererError::new("no config", Box::new(OpaqueError("".to_string()))))?
        };
        let nc = unsafe {
            display.create_context(&gl_config, &ContextAttributesBuilder::new()
                .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
                .build(Some(wh))
            ).map_err(|e| RendererError::new("context", Box::new(e)))?
        };
        let surf = unsafe {
            display.create_window_surface(&gl_config, &SurfaceAttributesBuilder::<WindowSurface>::new()
                .build(wh, NonZeroU32::new(config.initial_size.width.max(1)).unwrap(),
                       NonZeroU32::new(config.initial_size.height.max(1)).unwrap())
            ).map_err(|e| RendererError::new("surface", Box::new(e)))?
        };
        let ctx = nc.make_current(&surf).map_err(|e| RendererError::new("make_current", Box::new(e)))?;

        GlView::load_with(|n| {
            let c = std::ffi::CString::new(n).unwrap();
            display.get_proc_address(&c)
        });
        let mut view = GlView::new();
        view.use_vao(true);
        view.on_load(&config.render_profile).map_err(|e| RendererError::new("view", Box::new(OpaqueError(e))))?;

        let fs = match config.render_profile.frame_format {
            VideoFrameFormat::Rgba => config.render_profile.logical_size,
            VideoFrameFormat::Palette => config.render_profile.source_logical_size,
        };
        Ok(Box::new(GlRenderer {
            view, context: ctx, gl_surface: surf,
            expected_frame_len: fs.width * fs.height * config.render_profile.frame_format.bytes_per_pixel(),
            size: config.initial_size,
        }))
    }
}
