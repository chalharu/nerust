use nerust_console::video::VideoRenderProfile;
use nerust_screen_video::{VideoFrameFormat, VideoFrameSpec, VideoPresentation};
use nerust_screen_wgpu::renderer::{
    DeviceLimitProfile, PresentationOptions, RenderOutcome, Renderer,
};
use nerust_screen_wgpu::surface::{RenderSurface, SurfaceSize, SurfaceTargetSource};
use raw_window_handle::{HandleError, RawDisplayHandle, RawWindowHandle};

/// Shell-side contract for surfaces that can host a wgpu renderer.
///
/// # Safety
///
/// Implementors must ensure the returned raw display/window handles describe a
/// stable native surface target, and that the backing native objects outlive
/// any `RenderSurface` built from them.
pub unsafe trait RenderSurfaceTarget {
    fn prepare(&self);

    fn surface_size(&self, fallback: SurfaceSize) -> SurfaceSize;

    fn raw_window_handle(&self) -> Result<RawWindowHandle, HandleError>;

    fn raw_display_handle(&self) -> Result<Option<RawDisplayHandle>, HandleError>;
}

struct ShellSurfaceTarget<T>(T);

// Safety: `T` is required to uphold the `RenderSurfaceTarget` contract, and
// `RenderSurface` stores `Surface<'static>` ahead of the wrapped target so the
// wgpu surface is dropped before the native target handles it was built from.
unsafe impl<T: RenderSurfaceTarget> SurfaceTargetSource for ShellSurfaceTarget<T> {
    fn prepare(&self) {
        self.0.prepare();
    }

    fn surface_size(&self, fallback: SurfaceSize) -> SurfaceSize {
        self.0.surface_size(fallback)
    }

    fn raw_window_handle(&self) -> Result<RawWindowHandle, HandleError> {
        self.0.raw_window_handle()
    }

    fn raw_display_handle(&self) -> Result<Option<RawDisplayHandle>, HandleError> {
        self.0.raw_display_handle()
    }
}

/// Outcome reported to the shell after a [`WgpuBackend::render`] call.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum RenderResult {
    /// A frame was successfully presented.
    Presented,
    /// The frame was skipped (surface not ready, resize in flight, etc.).
    Skipped,
    /// A render error occurred; the shell may log or surface this as needed.
    Error,
}

/// App-facing wgpu render backend.
///
/// Owns the [`RenderSurface`] and [`Renderer`] lifecycle so that wgpu shells
/// are free from direct surface-management dependencies below the backend.
/// Surface recreation and reconfiguration after resize or loss are handled
/// transparently inside [`render`](Self::render).
pub struct WgpuBackend<T: RenderSurfaceTarget> {
    // Drop order matters: release GPU resources before tearing down the
    // surface/instance pair they were created against.
    renderer: Renderer,
    render_surface: RenderSurface<ShellSurfaceTarget<T>>,
    last_render_error: Option<String>,
}

impl<T: RenderSurfaceTarget> WgpuBackend<T> {
    /// Create a new backend from a shell-provided surface target.
    ///
    /// `initial_size` is used as a fallback when the surface target cannot
    /// determine its own size (e.g. before the first layout pass). The backend
    /// calls [`Renderer::new`] synchronously via `pollster::block_on`.
    pub fn new(
        target: T,
        initial_size: SurfaceSize,
        render_profile: &VideoRenderProfile,
        presentation_options: PresentationOptions,
    ) -> Result<Self, String> {
        Self::new_with_device_limit_profile(
            target,
            initial_size,
            render_profile,
            DeviceLimitProfile::Default,
            presentation_options,
        )
    }

    pub fn new_with_device_limit_profile(
        target: T,
        initial_size: SurfaceSize,
        render_profile: &VideoRenderProfile,
        device_limit_profile: DeviceLimitProfile,
        presentation_options: PresentationOptions,
    ) -> Result<Self, String> {
        let presentation = VideoPresentation::new(VideoFrameSpec::new(
            VideoFrameFormat::Rgba,
            render_profile.source_logical_size,
            render_profile.logical_size,
            render_profile.physical_size,
        ));
        let render_surface = RenderSurface::new(ShellSurfaceTarget(target))?;
        let surface_size = render_surface.surface_size(initial_size);
        let renderer = pollster::block_on(Renderer::new_with_device_limit_profile(
            &render_surface,
            surface_size,
            &presentation,
            None,
            device_limit_profile,
            presentation_options,
        ))?;
        Ok(Self {
            renderer,
            render_surface,
            last_render_error: None,
        })
    }

    /// Render a frame from `frame_buffer`.
    ///
    /// `window_size` is the current inner size of the OS window, used as a
    /// fallback if the surface target cannot determine its own size. Surface
    /// recreation on loss is handled internally; shells see
    /// [`RenderResult::Skipped`] for successful recovery and
    /// [`RenderResult::Error`] when recreation itself fails.
    pub fn render(&mut self, frame_buffer: &[u8], window_size: SurfaceSize) -> RenderResult {
        let surface_size = self.render_surface.surface_size(window_size);
        let outcome = self
            .renderer
            .render(&self.render_surface, surface_size, frame_buffer);
        match outcome {
            Ok(RenderOutcome::Presented) => {
                self.last_render_error = None;
                RenderResult::Presented
            }
            Ok(RenderOutcome::Skipped) => RenderResult::Skipped,
            Ok(RenderOutcome::RecreateSurface) => match self.render_surface.recreate_surface() {
                Ok(()) => {
                    self.last_render_error = None;
                    let new_size = self.render_surface.surface_size(window_size);
                    self.renderer
                        .reconfigure_surface(&self.render_surface, new_size);
                    RenderResult::Skipped
                }
                Err(err) => {
                    let should_log = self.last_render_error.as_deref() != Some(err.as_str());
                    self.last_render_error = Some(err.clone());
                    if should_log {
                        log::error!("wgpu surface recreation failed: {err}");
                    }
                    RenderResult::Error
                }
            },
            Err(err) => {
                let should_log = self.last_render_error.as_deref() != Some(err.as_str());
                self.last_render_error = Some(err.clone());
                if should_log {
                    log::error!("wgpu render error: {err}");
                }
                RenderResult::Error
            }
        }
    }

    /// Upload (or update) the palette texture used for PaletteDecode rendering.
    ///
    /// `palette_rgba8` must be PALETTE_TEXTURE_WIDTH × 4 bytes (64 RGBA texels).
    /// This is a no-op for the `DirectColor` pipeline (the data is simply ignored).
    pub fn update_palette_texture(&mut self, palette_rgba8: &[u8]) {
        self.renderer.update_palette_texture(palette_rgba8);
    }

    /// Reconfigure the wgpu surface for a new window size.
    ///
    /// Call this after a resize event, before the next [`render`](Self::render).
    pub fn reconfigure(&mut self, window_size: SurfaceSize) {
        let surface_size = self.render_surface.surface_size(window_size);
        self.renderer
            .reconfigure_surface(&self.render_surface, surface_size);
    }
}
