use std::sync::Mutex;

use nerust_screen_video::{
    FrameBuffer, OpaqueError, RenderResult, Renderer, RendererConfig, RendererError,
    RendererFactory, Surface, SurfaceSize, VideoFrameSpec, VideoPresentation, VideoRenderProfile,
};
use nerust_screen_wgpu::{
    renderer::{DeviceLimitProfile, PresentationOptions, RenderOutcome, RenderPipeline},
    surface,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

// ---------------------------------------------------------------------------
// WgpuRenderer  — GPU device + pipeline (implements Renderer)
// ---------------------------------------------------------------------------

/// Wgpu device + pipeline.  The actual wgpu::Device and Queue live inside
/// the RenderPipeline (accessible via pipeline.device() / pipeline.queue()).
pub struct WgpuRenderer {
    instance: wgpu::Instance,
    raw_window_handle: RawWindowHandle,
    raw_display_handle: RawDisplayHandle,
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
        let Some(wgpu_surf) = surface.downcast_ref::<WgpuSurface>() else {
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
        // Rebuild the pipeline with the new render profile.
        // We need a wgpu::Surface for RenderPipeline::new, so create a
        // temporary one from the stored raw handles.
        let temp_surface = surface::create_wgpu_surface(
            &self.instance,
            self.raw_window_handle,
            self.raw_display_handle,
        )
        .map_err(|e| RendererError::new("temp surface for rebuild", Box::new(OpaqueError(e))))?;

        let presentation = VideoPresentation::new(VideoFrameSpec::new(
            profile.frame_format,
            profile.source_logical_size,
            profile.logical_size,
            profile.physical_size,
        ));
        let current_size = SurfaceSize::new(
            self.pipeline.surface_config().width.max(1),
            self.pipeline.surface_config().height.max(1),
        );
        #[cfg(target_os = "android")]
        let device_limit = DeviceLimitProfile::DownlevelWebGl2;
        #[cfg(not(target_os = "android"))]
        let device_limit = DeviceLimitProfile::Default;

        self.pipeline = pollster::block_on(RenderPipeline::new(
            &self.instance,
            &temp_surface,
            current_size,
            &presentation,
            profile.ntsc_packed_rgba8.as_deref(),
            PresentationOptions {
                vsync: self.pipeline.surface_config().present_mode != wgpu::PresentMode::Fifo,
            },
            device_limit,
        ))
        .map_err(|e| RendererError::new("rebuild pipeline", Box::new(OpaqueError(e))))?;
        // temp_surface drops here — the WgpuSurface held by the frontend
        // remains valid and will be reused.
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WgpuSurface  — platform output (implements Surface)
// ---------------------------------------------------------------------------

/// Wgpu output surface + swapchain config.
pub struct WgpuSurface {
    instance: wgpu::Instance,
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
    pub fn wgpu_surface(&self) -> &wgpu::Surface<'static> {
        &self.wgpu_surface
    }
}

impl Surface for WgpuSurface {
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
        let wgpu_surface =
            surface::create_wgpu_surface(&self.instance, window_handle, display_handle)
                .map_err(|e| RendererError::new("recreate surface", Box::new(OpaqueError(e))))?;
        self.wgpu_surface = wgpu_surface;
        self.configure(size);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WgpuRendererFactory  — implements RendererFactory
// ---------------------------------------------------------------------------

/// Stateful factory: create_renderer stores the surface + config internally,
/// create_surface retrieves them.  This avoids requiring the frontend to
/// orchestrate two-phase creation for wgpu.
pub struct WgpuRendererFactory {
    state: Mutex<
        Option<(
            wgpu::Instance,
            wgpu::Surface<'static>,
            wgpu::SurfaceConfiguration,
            wgpu::Device,
        )>,
    >,
}

impl WgpuRendererFactory {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(None),
        }
    }
}

impl Default for WgpuRendererFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl WgpuRendererFactory {
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

        // Store surface + config for create_surface.
        *self.state.lock().unwrap() = Some((
            instance.clone(),
            wgpu_surface,
            surface_config,
            device.clone(),
        ));

        Ok(Box::new(WgpuRenderer {
            instance,
            raw_window_handle: window_handle,
            raw_display_handle: display_handle,
            pipeline,
            last_render_error: None,
        }))
    }

    fn create_surface(
        &self,
        renderer: &dyn Renderer,
        _window_handle: RawWindowHandle,
        _display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<Box<dyn Surface>, RendererError> {
        // Validate that the renderer is a WgpuRenderer.
        renderer.downcast_ref::<WgpuRenderer>().ok_or_else(|| {
            RendererError::new(
                "create_surface",
                Box::new(OpaqueError("renderer is not a WgpuRenderer".to_string())),
            )
        })?;

        let mut guard = self.state.lock().unwrap();
        let (instance, wgpu_surface, mut config, device) = guard.take().ok_or_else(|| {
            RendererError::new(
                "create_surface",
                Box::new(OpaqueError(
                    "create_renderer must be called first".to_string(),
                )),
            )
        })?;

        config.width = size.width.max(1);
        config.height = size.height.max(1);
        wgpu_surface.configure(&device, &config);

        Ok(Box::new(WgpuSurface {
            instance,
            wgpu_surface,
            config,
            device,
            size,
        }))
    }
}
