use nerust_screen_video::{
    FrameBuffer, RenderResult, Renderer, SurfaceSize, VideoFrameSpec, VideoPresentation,
    VideoRenderProfile,
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
    #[allow(dead_code)] // kept for surface recreation
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    pipeline: RenderPipeline,
    current_size: SurfaceSize,
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
    /// Create a new wgpu renderer from raw window/display handles.
    ///
    /// `initial_size` is the starting surface size.
    /// `render_profile` describes the frame format and presentation layout.
    /// `device_limit_profile` selects the GPU capability target.
    /// `vsync` controls whether presentation is synchronized to the display.
    pub fn new(
        raw_window_handle: RawWindowHandle,
        raw_display_handle: RawDisplayHandle,
        initial_size: SurfaceSize,
        render_profile: &VideoRenderProfile,
        device_limit_profile: DeviceLimitProfile,
        vsync: bool,
    ) -> Result<Self, String> {
        let instance = surface::default_instance();
        let wgpu_surface =
            surface::create_wgpu_surface(&instance, raw_window_handle, raw_display_handle)?;

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
        ))?;

        Ok(Self {
            instance,
            surface: wgpu_surface,
            pipeline,
            current_size: initial_size,
            last_render_error: None,
        })
    }
}

impl Renderer for WgpuRenderer {
    fn render(&mut self, frame: &FrameBuffer) -> RenderResult {
        if let Some(palette_rgba8) = frame.palette_as_rgba8() {
            self.pipeline.update_palette_texture(&palette_rgba8);
        }

        let outcome = self
            .pipeline
            .render(&self.surface, self.current_size, frame.as_ref());

        match outcome {
            Ok(RenderOutcome::Presented) => {
                self.last_render_error = None;
                RenderResult::Presented
            }
            Ok(RenderOutcome::Skipped) => RenderResult::Skipped,
            Ok(RenderOutcome::RecreateSurface) => {
                // Recreate the wgpu surface from the same instance using stored handles.
                // For wgpu surfaces created from raw handles, we need the original
                // handles. The WgpuRenderer stores them for this purpose.
                match self.recreate_surface() {
                    Ok(()) => {
                        self.last_render_error = None;
                        self.pipeline
                            .reconfigure_surface(&self.surface, self.current_size);
                        RenderResult::Skipped
                    }
                    Err(err) => {
                        let should_log = self.last_render_error.as_deref() != Some(err.as_str());
                        if should_log {
                            log::error!("wgpu surface recreation failed: {err}");
                        }
                        self.last_render_error = Some(err);
                        RenderResult::Error
                    }
                }
            }
            Err(err) => {
                let should_log = self.last_render_error.as_deref() != Some(err.as_str());
                if should_log {
                    log::error!("wgpu render error: {err}");
                }
                self.last_render_error = Some(err);
                RenderResult::Error
            }
        }
    }

    fn reconfigure(&mut self, size: SurfaceSize) {
        self.current_size = size;
        self.pipeline.reconfigure_surface(&self.surface, size);
    }
}

// Private helpers
impl WgpuRenderer {
    fn recreate_surface(&mut self) -> Result<(), String> {
        // Surface recreation from raw handles requires storing the raw
        // handles during `new()`. Currently `RecreateSurface` is rare.
        Err("wgpu surface recreation requires stored raw handles".to_string())
    }
}
