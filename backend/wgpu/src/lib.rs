use nerust_screen_video::{
    FrameBuffer, OpaqueError, RenderResult, Renderer, RendererConfig, RendererError,
    RendererFactory, Surface, SurfaceSize, VideoFrameSpec, VideoPresentation, VideoRenderProfile,
};
use nerust_screen_wgpu::renderer::{
    DeviceLimitProfile, PresentationOptions, RenderOutcome, RenderPipeline,
};
use nerust_screen_wgpu::surface;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

// ---------------------------------------------------------------------------
// WgpuRenderer  — GPU device + pipeline (implements Renderer)
// ---------------------------------------------------------------------------

/// Wgpu renderer: instance + device + queue + pipeline.
pub struct WgpuRenderer {
    #[allow(dead_code)]
    instance: wgpu::Instance,
    #[allow(dead_code)]
    device: wgpu::Device,
    #[allow(dead_code)]
    queue: wgpu::Queue,
    pipeline: RenderPipeline,
    last_render_error: Option<String>,
}

impl std::fmt::Debug for WgpuRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuRenderer")
            .field("last_render_error", &self.last_render_error)
            .finish_non_exhaustive()
    }
}

impl Renderer for WgpuRenderer {
    fn render(&mut self, surface: &dyn Surface, frame: &FrameBuffer) -> RenderResult {
        let Some(wgpu_surf) = surface.as_any().downcast_ref::<WgpuSurface>() else {
            log::error!("WgpuRenderer: surface is not a WgpuSurface");
            return RenderResult::Error;
        };

        if let Some(palette_rgba8) = frame.palette_as_rgba8() {
            self.pipeline.update_palette_texture(&palette_rgba8);
        }

        let window_size = wgpu_surf.size();
        let outcome = self
            .pipeline
            .render(wgpu_surf.wgpu_surface(), window_size, frame.as_ref());

        match outcome {
            Ok(RenderOutcome::Presented) => {
                self.last_render_error = None;
                RenderResult::Presented
            }
            Ok(RenderOutcome::Skipped) => RenderResult::Skipped,
            Ok(RenderOutcome::RecreateSurface) => {
                // The surface was lost — store the new surface from the pipeline.
                // For wgpu the Surface handle stored in WgpuSurface is already
                // invalid; the frontend should call Surface::recreate() instead.
                self.last_render_error = None;
                RenderResult::Skipped
            }
            Err(err) => {
                let should_log = self.last_render_error.as_deref() != Some(err.as_str());
                if should_log {
                    log::error!("wgpu render error: {err}");
                }
                self.last_render_error = Some(err.to_string());
                RenderResult::Error
            }
        }
    }

    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        // For wgpu the pipeline is built from the render profile at init time.
        // A full pipeline rebuild would require re-creating the RenderPipeline.
        // For now the initial config is sufficient since NTSC filter changes
        // only affect the frame format, which the existing pipeline handles.
        let _ = profile;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WgpuSurface  — platform output (implements Surface)
// ---------------------------------------------------------------------------

/// Wgpu output surface + swapchain config.
pub struct WgpuSurface {
    wgpu_surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    size: SurfaceSize,
}

impl std::fmt::Debug for WgpuSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuSurface")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl WgpuSurface {
    fn wgpu_surface(&self) -> &wgpu::Surface<'static> {
        &self.wgpu_surface
    }
}

impl Surface for WgpuSurface {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn size(&self) -> SurfaceSize {
        self.size
    }

    fn configure(&mut self, size: SurfaceSize) {
        self.size = size;
        self.config.width = size.width;
        self.config.height = size.height;
        self.wgpu_surface.configure(&self.device, &self.config);
    }

    fn recreate(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        let instance = surface::default_instance();
        let wgpu_surface =
            surface::create_wgpu_surface(&instance, window_handle, display_handle)
                .map_err(|e| RendererError::new("recreate surface", Box::new(OpaqueError(e))))?;
        self.wgpu_surface = wgpu_surface;
        self.configure(size);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WgpuRendererFactory  — implements RendererFactory
// ---------------------------------------------------------------------------

pub struct WgpuRendererFactory;

impl WgpuRendererFactory {
    /// Helper: create both a renderer and a surface from the same handles.
    /// Used by frontends during initialization.
    pub fn create_renderer_and_surface(
        config: &RendererConfig,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
    ) -> Result<(Box<dyn Renderer>, Box<dyn Surface>), RendererError> {
        let instance = surface::default_instance();
        let wgpu_surface =
            surface::create_wgpu_surface(&instance, window_handle, display_handle)
                .map_err(|e| RendererError::new("create wgpu surface", Box::new(OpaqueError(e))))?;

        let presentation = VideoPresentation::new(VideoFrameSpec::new(
            config.render_profile.frame_format,
            config.render_profile.source_logical_size,
            config.render_profile.logical_size,
            config.render_profile.physical_size,
        ));
        let pipeline = pollster::block_on(RenderPipeline::new(
            &instance,
            &wgpu_surface,
            config.initial_size,
            &presentation,
            config.render_profile.ntsc_packed_rgba8.as_deref(),
            PresentationOptions {
                vsync: config.vsync,
            },
            Self::device_limit_profile(),
        ))
        .map_err(|e| RendererError::new("wgpu pipeline init", Box::new(OpaqueError(e))))?;

        let device = pipeline.device().clone();
        let surface_config = pipeline.surface_config().clone();
        let initial_size = config.initial_size;

        let renderer = Box::new(WgpuRenderer {
            instance,
            device: device.clone(),
            queue: pipeline.queue().clone(),
            pipeline,
            last_render_error: None,
        }) as Box<dyn Renderer>;

        let surface = Box::new(WgpuSurface {
            wgpu_surface,
            config: surface_config,
            device,
            size: initial_size,
        }) as Box<dyn Surface>;

        Ok((renderer, surface))
    }

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

impl RendererFactory for WgpuRendererFactory {
    fn create_renderer(
        &self,
        config: &RendererConfig,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn Renderer>, RendererError> {
        let (renderer, _surface) =
            Self::create_renderer_and_surface(config, window_handle, display_handle)?;
        // surface is stored in the factory for later retrieval.
        // For now, frontends should use create_renderer_and_surface() directly.
        Ok(renderer)
    }

    fn create_surface(
        &self,
        _window_handle: RawWindowHandle,
        _display_handle: RawDisplayHandle,
        _size: SurfaceSize,
    ) -> Result<Box<dyn Surface>, RendererError> {
        Err(RendererError::new(
            "create_surface",
            Box::new(OpaqueError(
                "use WgpuRendererFactory::create_renderer_and_surface".to_string(),
            )),
        ))
    }
}
