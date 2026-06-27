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
    render_profile: VideoRenderProfile,
    pipeline: Option<RenderPipeline>,
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

impl WgpuRenderer {
    fn build_pipeline(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        size: SurfaceSize,
        profile: &VideoRenderProfile,
        vsync: bool,
    ) -> Result<RenderPipeline, RendererError> {
        let presentation = VideoPresentation::new(VideoFrameSpec::new(
            profile.frame_format,
            profile.source_logical_size,
            profile.logical_size,
            profile.physical_size,
        ));
        #[cfg(target_os = "android")]
        let dl = DeviceLimitProfile::DownlevelWebGl2;
        #[cfg(not(target_os = "android"))]
        let dl = DeviceLimitProfile::Default;
        pollster::block_on(RenderPipeline::new(
            instance,
            surface,
            size,
            &presentation,
            profile.ntsc_packed_rgba8.as_deref(),
            PresentationOptions { vsync },
            dl,
        ))
        .map_err(|e| RendererError::new("pipeline", Box::new(OpaqueError(e))))
    }
}

impl GpuRenderer for WgpuRenderer {
    fn size(&self) -> SurfaceSize {
        self.size
    }

    fn attach(
        &mut self,
        wh: RawWindowHandle,
        dh: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.size = size;
        let wgpu_surface = surface::create_wgpu_surface(&self.instance, wh, dh)
            .map_err(|e| RendererError::new("surface", Box::new(OpaqueError(e))))?;
        let pipeline = Self::build_pipeline(
            &self.instance,
            &wgpu_surface,
            size,
            &self.render_profile,
            true,
        )?;
        self.surface = Some(wgpu_surface);
        self.pipeline = Some(pipeline);
        Ok(())
    }

    fn detach(&mut self) {
        self.surface = None;
        self.pipeline = None;
    }

    fn resize(&mut self, size: SurfaceSize) {
        self.size = size;
        let Some(ref surface) = self.surface else {
            return;
        };
        let Some(ref mut pipeline) = self.pipeline else {
            return;
        };
        pipeline.reconfigure_surface(surface, size);
    }

    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        let Some(ref surface) = self.surface else {
            return Err(RendererError::new(
                "update: not attached",
                Box::new(OpaqueError("".to_string())),
            ));
        };
        self.pipeline = Some(Self::build_pipeline(
            &self.instance,
            surface,
            self.size,
            profile,
            true,
        )?);
        Ok(())
    }

    fn render(&mut self, frame: &FrameBuffer) -> RenderResult {
        let Some(ref surface) = self.surface else {
            return RenderResult::Skipped;
        };
        let Some(ref mut pipeline) = self.pipeline else {
            return RenderResult::Skipped;
        };

        if let Some(p) = frame.palette_as_rgba8() {
            pipeline.update_palette_texture(&p);
        }

        match pipeline.render(surface, self.size, frame.as_ref()) {
            Ok(RenderOutcome::Presented) => {
                self.last_render_error = None;
                RenderResult::Presented
            }
            Ok(RenderOutcome::Skipped) => RenderResult::Skipped,
            Ok(RenderOutcome::RecreateSurface) => {
                self.last_render_error = None;
                RenderResult::Skipped
            }
            Err(e) => {
                if self.last_render_error.as_deref() != Some(e.as_str()) {
                    log::error!("wgpu: {e}");
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
        _display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError> {
        let instance = surface::default_instance();

        // Create a headless device + queue.  The pipeline and surface are
        // created in attach() where we have actual window handles.
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|e| RendererError::new("request adapter", Box::new(OpaqueError(e.to_string()))))?;
        let limits = Self::device_limit_profile().required_limits();
        let (_device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("nerust_wgpu_device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                ..Default::default()
            }))
            .map_err(|e| RendererError::new("device", Box::new(OpaqueError(e.to_string()))))?;

        Ok(Box::new(WgpuRenderer {
            instance,
            render_profile: config.render_profile.clone(),
            pipeline: None,
            surface: None,
            size: config.initial_size,
            last_render_error: None,
        }))
    }
}
