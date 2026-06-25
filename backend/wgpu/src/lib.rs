use nerust_screen_video::{
    FrameBuffer, OpaqueError, RenderResult, Renderer, RendererConfig, RendererError,
    RendererFactory, SurfaceSize, VideoFrameSpec, VideoPresentation, VideoRenderProfile,
};
use nerust_screen_wgpu::renderer::{
    DeviceLimitProfile, PresentationOptions, RenderOutcome, RenderPipeline,
};
use nerust_screen_wgpu::surface;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Wgpu-based [`Renderer`] implementation.
///
/// Owns the wgpu instance, surface, and render pipeline. Created from a
/// window/display handle pair so it is completely decoupled from any
/// specific windowing toolkit.
pub struct WgpuRenderer {
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    pipeline: RenderPipeline,
    current_size: SurfaceSize,
    raw_window_handle: RawWindowHandle,
    raw_display_handle: RawDisplayHandle,
    last_render_error: Option<String>,
}

impl std::fmt::Debug for WgpuRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuRenderer")
            .field("current_size", &self.current_size)
            .field("last_render_error", &self.last_render_error)
            .finish_non_exhaustive()
    }
}

impl WgpuRenderer {
    fn new(
        raw_window_handle: RawWindowHandle,
        raw_display_handle: RawDisplayHandle,
        initial_size: SurfaceSize,
        render_profile: &VideoRenderProfile,
        device_limit_profile: DeviceLimitProfile,
        vsync: bool,
    ) -> Result<Self, RendererError> {
        let instance = surface::default_instance();
        let wgpu_surface =
            surface::create_wgpu_surface(&instance, raw_window_handle, raw_display_handle)
                .map_err(|e| RendererError::new("create wgpu surface", Box::new(OpaqueError(e))))?;

        let presentation = VideoPresentation::new(VideoFrameSpec::new(
            render_profile.frame_format,
            render_profile.source_logical_size,
            render_profile.logical_size,
            render_profile.physical_size,
        ));
        let pipeline = pollster::block_on(RenderPipeline::new(
            &instance,
            &wgpu_surface,
            initial_size,
            &presentation,
            render_profile.ntsc_packed_rgba8.as_deref(),
            PresentationOptions { vsync },
            device_limit_profile,
        ))
        .map_err(|e| RendererError::new("wgpu pipeline init", Box::new(OpaqueError(e))))?;

        Ok(Self {
            instance,
            surface: wgpu_surface,
            pipeline,
            current_size: initial_size,
            raw_window_handle,
            raw_display_handle,
            last_render_error: None,
        })
    }
}

impl Renderer for WgpuRenderer {
    fn render(&mut self, frame: &FrameBuffer, window_size: SurfaceSize) -> RenderResult {
        if let Some(palette_rgba8) = frame.palette_as_rgba8() {
            self.pipeline.update_palette_texture(&palette_rgba8);
        }

        self.current_size = window_size;
        let outcome = self
            .pipeline
            .render(&self.surface, window_size, frame.as_ref());

        match outcome {
            Ok(RenderOutcome::Presented) => {
                self.last_render_error = None;
                RenderResult::Presented
            }
            Ok(RenderOutcome::Skipped) => RenderResult::Skipped,
            Ok(RenderOutcome::RecreateSurface) => match self.do_recreate_surface() {
                Ok(()) => {
                    self.last_render_error = None;
                    self.pipeline
                        .reconfigure_surface(&self.surface, self.current_size);
                    RenderResult::Skipped
                }
                Err(err) => {
                    let err_str = err.to_string();
                    let should_log = self.last_render_error.as_deref() != Some(&err_str);
                    if should_log {
                        log::error!("wgpu surface recreation failed: {err}");
                    }
                    self.last_render_error = Some(err_str);
                    RenderResult::Error
                }
            },
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

    fn reconfigure(&mut self, size: SurfaceSize) {
        self.current_size = size;
        self.pipeline.reconfigure_surface(&self.surface, size);
    }

    fn recreate_surface(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.raw_window_handle = window_handle;
        self.raw_display_handle = display_handle;
        self.current_size = size;
        self.do_recreate_surface()
    }
}

// Private helpers
impl WgpuRenderer {
    fn do_recreate_surface(&mut self) -> Result<(), RendererError> {
        self.surface = surface::create_wgpu_surface(
            &self.instance,
            self.raw_window_handle,
            self.raw_display_handle,
        )
        .map_err(|e| RendererError::new("recreate surface", Box::new(OpaqueError(e))))?;
        self.pipeline
            .reconfigure_surface(&self.surface, self.current_size);
        Ok(())
    }
}

/// `WgpuRenderer` を構築する Factory。
pub struct WgpuRendererFactory;

impl RendererFactory for WgpuRendererFactory {
    fn create_renderer(
        &self,
        config: &RendererConfig,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn Renderer>, RendererError> {
        #[cfg(target_os = "android")]
        let profile = DeviceLimitProfile::DownlevelWebGl2;
        #[cfg(not(target_os = "android"))]
        let profile = DeviceLimitProfile::Default;

        WgpuRenderer::new(
            window_handle,
            display_handle,
            config.initial_size,
            &config.render_profile,
            profile,
            config.vsync,
        )
        .map(|r| Box::new(r) as Box<dyn Renderer>)
    }
}
