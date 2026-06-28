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
use nerust_screen_opengl::GlView;
use nerust_screen_video::{
    FrameBuffer, GpuFactory, GpuRenderer, OpaqueError, RenderResult, RendererConfig, RendererError,
    SurfaceSize, VideoFrameFormat, VideoRenderProfile,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

pub struct GlRenderer {
    display: Display,
    gl_config: glutin::config::Config,
    render_profile: VideoRenderProfile,
    view: Option<GlView>,
    context: Option<glutin::context::PossiblyCurrentContext>,
    gl_surface: Option<glutin::surface::Surface<WindowSurface>>,
    expected_frame_len: usize,
    size: SurfaceSize,
}

impl std::fmt::Debug for GlRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlRenderer")
            .field("attached", &self.gl_surface.is_some())
            .finish_non_exhaustive()
    }
}

impl GpuRenderer for GlRenderer {
    fn size(&self) -> SurfaceSize {
        self.size
    }

    fn resize(&mut self, size: SurfaceSize) {
        if let Some(ref surf) = self.gl_surface {
            let (w, h) = (
                NonZeroU32::new(size.width.max(1)).unwrap(),
                NonZeroU32::new(size.height.max(1)).unwrap(),
            );
            surf.resize(self.context.as_ref().unwrap(), w, h);
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
            NonZeroU32::new(size.width.max(1)).unwrap(),
            NonZeroU32::new(size.height.max(1)).unwrap(),
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

        // Compile shaders.
        GlView::load_with(|n| {
            let c = std::ffi::CString::new(n).unwrap();
            self.display.get_proc_address(&c)
        });
        let mut view = GlView::new();
        view.use_vao(true);
        view.on_load(&self.render_profile)
            .map_err(|e| RendererError::new("view", Box::new(OpaqueError(e))))?;
        let fs = match self.render_profile.frame_format {
            VideoFrameFormat::Rgba => self.render_profile.logical_size,
            VideoFrameFormat::Palette => self.render_profile.source_logical_size,
        };

        self.view = Some(view);
        self.context = Some(ctx);
        self.gl_surface = Some(surf);
        self.expected_frame_len =
            fs.width * fs.height * self.render_profile.frame_format.bytes_per_pixel();
        Ok(())
    }

    fn detach(&mut self) {
        self.view = None;
        self.context = None;
        drop(self.gl_surface.take());
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
        if let Some(ref mut v) = self.view {
            v.on_close();
        }
        let mut view = GlView::new();
        view.use_vao(true);
        view.on_load(profile)
            .map_err(|e| RendererError::new("view", Box::new(OpaqueError(e))))?;
        let fs = match profile.frame_format {
            VideoFrameFormat::Rgba => profile.logical_size,
            VideoFrameFormat::Palette => profile.source_logical_size,
        };
        self.expected_frame_len = fs.width * fs.height * profile.frame_format.bytes_per_pixel();
        self.view = Some(view);
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

        view.on_resize(self.size.width as i32, self.size.height as i32);
        if let Some(p) = frame_buffer.palette_as_rgba8() {
            view.update_palette_texture(&p);
        }
        let bytes = frame_buffer.as_ref();
        let bytes = bytes
            .get(..self.expected_frame_len)
            .expect("frame buffer too small");
        view.on_update(bytes.as_ptr());
        if let Err(e) = surf.swap_buffers(ctx) {
            log::warn!("GlRenderer: swap_buffers failed: {e}");
            return RenderResult::Error;
        }
        RenderResult::Presented
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
        }))
    }
}
