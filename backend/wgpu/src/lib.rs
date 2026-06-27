use nerust_screen_video::{
    FrameBuffer, GpuFactory, GpuRenderer, OpaqueError, RenderResult, RendererConfig, RendererError,
    SurfaceSize, VideoFrameSpec, VideoPresentation, VideoRenderProfile,
};
use nerust_screen_wgpu::renderer::{
    DeviceLimitProfile, PresentationOptions, RenderOutcome, RenderPipeline,
};
use nerust_screen_wgpu::surface;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

// ---------------------------------------------------------------------------
// WgpuRenderer
// ---------------------------------------------------------------------------

/// Wgpu device + pipeline.  Surface is attached/detached dynamically.
pub struct WgpuRenderer {
    instance: wgpu::Instance,
    pipeline: RenderPipeline,
    raw_window_handle: RawWindowHandle,
    raw_display_handle: RawDisplayHandle,
    surface: Option<wgpu::Surface<'static>>,
    size: SurfaceSize,
    last_render_error: Option<String>,
}

impl std::fmt::Debug for WgpuRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuRenderer")
            .field("size", &self.size)
            .field("last_render_error", &self.last_render_error)
            .finish_non_exhaustive()
    }
}

impl GpuRenderer for WgpuRenderer {
    fn size(&self) -> SurfaceSize {
        self.size
    }

    fn attach(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.raw_window_handle = window_handle;
        self.raw_display_handle = display_handle;
        self.size = size;

        let surface = surface::create_wgpu_surface(&self.instance, window_handle, display_handle)
            .map_err(|e| {
            RendererError::new("attach: create surface", Box::new(OpaqueError(e)))
        })?;

        let mut config = self.pipeline.surface_config().clone();
        config.width = size.width.max(1);
        config.height = size.height.max(1);
        surface.configure(self.pipeline.device(), &config);

        self.surface = Some(surface);
        Ok(())
    }

    fn detach(&mut self) {
        self.surface = None;
    }

    fn resize(&mut self, size: SurfaceSize) {
        self.size = size;
        let Some(ref surface) = self.surface else {
            return;
        };
        let mut config = self.pipeline.surface_config().clone();
        config.width = size.width.max(1);
        config.height = size.height.max(1);
        surface.configure(self.pipeline.device(), &config);
    }

    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        let presentation = VideoPresentation::new(VideoFrameSpec::new(
            profile.frame_format,
            profile.source_logical_size,
            profile.logical_size,
            profile.physical_size,
        ));
        let temp_surface = surface::create_wgpu_surface(
            &self.instance,
            self.raw_window_handle,
            self.raw_display_handle,
        )
        .map_err(|e| RendererError::new("temp surface", Box::new(OpaqueError(e))))?;

        #[cfg(target_os = "android")]
        let device_limit = DeviceLimitProfile::DownlevelWebGl2;
        #[cfg(not(target_os = "android"))]
        let device_limit = DeviceLimitProfile::Default;

        self.pipeline = pollster::block_on(RenderPipeline::new(
            &self.instance,
            &temp_surface,
            self.size,
            &presentation,
            profile.ntsc_packed_rgba8.as_deref(),
            PresentationOptions { vsync: true },
            device_limit,
        ))
        .map_err(|e| RendererError::new("rebuild pipeline", Box::new(OpaqueError(e))))?;
        Ok(())
    }

    fn render(&mut self, frame: &FrameBuffer) -> RenderResult {
        let Some(ref surface) = self.surface else {
            log::warn!("WgpuRenderer: render called without attach");
            return RenderResult::Skipped;
        };

        if let Some(palette_rgba8) = frame.palette_as_rgba8() {
            self.pipeline.update_palette_texture(&palette_rgba8);
        }

        match self.pipeline.render(surface, self.size, frame.as_ref()) {
            Ok(RenderOutcome::Presented) => {
                self.last_render_error = None;
                RenderResult::Presented
            }
            Ok(RenderOutcome::Skipped) => RenderResult::Skipped,
            Ok(RenderOutcome::RecreateSurface) => {
                // The frontend should call reattach() for actual recovery.
                self.last_render_error = None;
                RenderResult::Skipped
            }
            Err(e) => {
                let should_log = self.last_render_error.as_deref() != Some(e.as_str());
                if should_log {
                    log::error!("wgpu render error: {e}");
                }
                self.last_render_error = Some(e.to_string());
                RenderResult::Error
            }
        }
    }
}

// ---------------------------------------------------------------------------
// WgpuFactory
// ---------------------------------------------------------------------------

pub struct WgpuFactory;

impl Default for WgpuFactory {
    fn default() -> Self {
        Self
    }
}

impl WgpuFactory {
    fn device_limit_profile() -> DeviceLimitProfile {
        #[cfg(target_os = "android")]
        {
            DeviceLimitProfile::DownlevelWebGl2
        }
        #[cfg(not(target_os = "android"))]
        {
            DeviceLimitProfile::Default
        }
    }
}

impl GpuFactory for WgpuFactory {
    fn create_renderer(
        &self,
        config: &RendererConfig,
        _window_handle: RawWindowHandle,
        _display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError> {
        let instance = surface::default_instance();

        // Create a temporary surface just for pipeline construction.
        // The real surface will be created in attach().
        let temp = surface::create_wgpu_surface(&instance, _window_handle, _display_handle)
            .map_err(|e| RendererError::new("temp surface", Box::new(OpaqueError(e))))?;

        let presentation = VideoPresentation::new(VideoFrameSpec::new(
            config.render_profile.frame_format,
            config.render_profile.source_logical_size,
            config.render_profile.logical_size,
            config.render_profile.physical_size,
        ));
        let pipeline = pollster::block_on(RenderPipeline::new(
            &instance,
            &temp,
            config.initial_size,
            &presentation,
            config.render_profile.ntsc_packed_rgba8.as_deref(),
            PresentationOptions {
                vsync: config.vsync,
            },
            Self::device_limit_profile(),
        ))
        .map_err(|e| RendererError::new("pipeline init", Box::new(OpaqueError(e))))?;

        Ok(Box::new(WgpuRenderer {
            instance,
            pipeline,
            raw_window_handle: _window_handle,
            raw_display_handle: _display_handle,
            surface: None,
            size: config.initial_size,
            last_render_error: None,
        }))
    }
}
