use std::sync::Mutex;

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

/// Wgpu device + pipeline.  The actual wgpu::Device and Queue live inside
/// the RenderPipeline (accessible via pipeline.device() / pipeline.queue()).
pub struct WgpuRenderer {
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
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

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

    fn update_render_profile(
        &mut self,
        _profile: &VideoRenderProfile,
    ) -> Result<(), RendererError> {
        // The wgpu pipeline is built with both palette and NTSC decode
        // capabilities, so a render profile change (e.g. NTSC filter toggle)
        // does not require a pipeline rebuild.
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
    pub fn wgpu_surface(&self) -> &wgpu::Surface<'static> {
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

/// Stateful factory: create_renderer stores the surface + config internally,
/// create_surface retrieves them.  This avoids requiring the frontend to
/// orchestrate two-phase creation for wgpu.
pub struct WgpuRendererFactory {
    state: Mutex<
        Option<(
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
        *self.state.lock().unwrap() = Some((wgpu_surface, surface_config, device.clone()));

        Ok(Box::new(WgpuRenderer {
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
        renderer
            .as_any()
            .downcast_ref::<WgpuRenderer>()
            .ok_or_else(|| {
                RendererError::new(
                    "create_surface",
                    Box::new(OpaqueError("renderer is not a WgpuRenderer".to_string())),
                )
            })?;

        let mut guard = self.state.lock().unwrap();
        let (wgpu_surface, mut config, device) = guard.take().ok_or_else(|| {
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
            wgpu_surface,
            config,
            device,
            size,
        }))
    }
}
