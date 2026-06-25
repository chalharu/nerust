use crate::{FrameBuffer, SurfaceSize, VideoRenderProfile};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Wraps a static or formatted message as an `std::error::Error`.
#[derive(Debug)]
pub struct OpaqueError(pub String);

impl std::fmt::Display for OpaqueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for OpaqueError {}

/// Renderer-related error.
///
/// `context` identifies the failing operation ("display init", "surface recreate"),
/// `source` carries the original typed error.
#[derive(Debug)]
pub struct RendererError {
    context: &'static str,
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl RendererError {
    pub fn new(context: &'static str, source: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self { context, source }
    }
}

impl std::fmt::Display for RendererError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.context, self.source)
    }
}

impl std::error::Error for RendererError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

/// Outcome reported by [`Renderer::render`].
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RenderResult {
    /// A frame was successfully presented.
    Presented,
    /// The frame was skipped (surface not ready, resize in flight, etc.).
    Skipped,
    /// A render error occurred; the shell may log or surface this as needed.
    Error,
}

/// GPU device + pipeline.  Lives for the entire application session.
pub trait Renderer: std::fmt::Debug {
    /// Render `frame` into the platform `surface`.
    /// Viewport and aspect-ratio are already configured by `Surface::configure`.
    fn render(&mut self, surface: &dyn Surface, frame: &FrameBuffer) -> RenderResult;

    /// Update the render pipeline for a new render profile
    /// (e.g. after an NTSC filter change).  The GPU device is kept.
    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError>;
}

/// Platform output (wgpu surface + swapchain / GL drawable).
///
/// Created by [`RendererFactory::create_surface`] and replaced on resize
/// or surface loss.  Independent of the [`Renderer`] GPU device.
pub trait Surface: std::fmt::Debug {
    /// Current physical pixel size of the platform output.
    fn size(&self) -> SurfaceSize;

    /// Resize the output (swapchain for wgpu; store for GL, applied at render time).
    fn configure(&mut self, size: SurfaceSize);

    /// Recreate the native platform surface after loss (Android).
    fn recreate(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError>;
}

/// Common parameters to create a [`Renderer`].
pub struct RendererConfig {
    pub initial_size: SurfaceSize,
    pub render_profile: VideoRenderProfile,
    pub vsync: bool,
}

/// Abstract factory: creates a [`Renderer`] and a [`Surface`] from the same backend family.
///
/// `create_renderer` takes raw window/display handles because OpenGL (glutin)
/// requires them to create a context.  wgpu ignores them and can create a
/// headless device.  Both backends keep the handles separate from the output
/// surface, which is managed via [`create_surface`](Self::create_surface).
pub trait RendererFactory {
    /// Build the GPU device + pipeline.
    /// `_window_handle` / `_display_handle` are required by GL; wgpu ignores them.
    fn create_renderer(
        &self,
        config: &RendererConfig,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn Renderer>, RendererError>;

    /// Build a platform output from native window/display handles.
    fn create_surface(
        &self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<Box<dyn Surface>, RendererError>;
}
